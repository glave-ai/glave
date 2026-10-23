import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';

import { TOKEN_KEY } from '../lib/token';
import {
  ELSEWHERE,
  FAKE_SERVER,
  FakeSession,
  SIGNED_IN,
  provideFakeSession,
} from '../testing/fake-session';
import { INITIAL_SESSION, SESSION_CLIENT_FACTORY, SessionService } from './session.service';

/** A tab that starts signed out, its calls going to `fake`, whose token each call is sent with. */
function startingOut(fake: FakeSession): { readonly token: () => string } {
  let sent: () => string = () => '';
  TestBed.configureTestingModule({
    providers: [
      {
        provide: SESSION_CLIENT_FACTORY,
        useValue: (token: () => string) => {
          sent = token;
          return fake;
        },
      },
      { provide: INITIAL_SESSION, useValue: null },
    ],
  });
  return { token: () => sent() };
}

const DISCOVERED = { name: 'glave', id: FAKE_SERVER };

describe('SessionService', () => {
  afterEach(() => {
    localStorage.clear();
    history.replaceState(null, '', '/');
  });

  it('is signed in with a session, and calls through the client', async () => {
    const fake = new FakeSession().answer('server/discover', { name: 'glave' });
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const session = TestBed.inject(SessionService);
    expect(session.signedIn()).toBe(true);
    expect(session.local()).toBe(true);
    expect(await session.call('server/discover', {})).toEqual({ name: 'glave' });
  });

  it('signs out when the server refuses the sign-in', async () => {
    const fake = new FakeSession().answer(
      'project/list',
      new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'no' }),
    );
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const session = TestBed.inject(SessionService);
    await expect(session.call('project/list', {})).rejects.toBeInstanceOf(ApiError);
    expect(session.signedIn()).toBe(false);
    expect(localStorage.getItem(TOKEN_KEY)).toBeNull();
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

  it("trades the link's code for a session with the server that served the app", async () => {
    history.replaceState(null, '', '/t/1#code=abc');
    const fake = new FakeSession()
      .answer('server/discover', DISCOVERED)
      .answer('signin/redeem', { session: 'glave_session_new', server: FAKE_SERVER });
    const sent = startingOut(fake);
    const session = TestBed.inject(SessionService);
    await session.start();
    expect(location.hash).toBe('');
    expect(location.pathname).toBe('/t/1');
    expect(fake.paramsOf('signin/redeem')).toEqual([{ code: 'abc' }]);
    expect(session.signedIn()).toBe(true);
    expect(sent.token()).toBe('glave_session_new');
    expect(JSON.parse(localStorage.getItem(TOKEN_KEY) ?? 'null')).toEqual({
      token: 'glave_session_new',
      server: FAKE_SERVER,
    });
  });

  it('makes passkeys at the public origin, or here at this page once the server says so', async () => {
    const cases = [
      [{ origin: 'https://glave.test', signin: ['link', 'passkey'] }, [true, true, false]],
      [{ here: window.location.origin, signin: ['link'] }, [true, false, true]],
      [{ here: window.location.origin, signin: ['link', 'passkey'] }, [true, true, true]],
      [{ here: 'http://localhost:1', signin: ['link', 'passkey'] }, [false, false, false]],
      [{ signin: ['link'] }, [false, false, false]],
    ] as const;
    for (const [discovered, [passkeys, signIn, here]] of cases) {
      TestBed.resetTestingModule();
      startingOut(new FakeSession().answer('server/discover', { ...DISCOVERED, ...discovered }));
      const session = TestBed.inject(SessionService);
      await session.start();
      const said = JSON.stringify(discovered);
      expect(session.passkeys(), said).toBe(passkeys);
      expect(session.passkeySignIn(), said).toBe(signIn);
      expect(session.passkeysHere(), said).toBe(here);
    }
    const session = TestBed.inject(SessionService);
    session.madePasskey();
    expect(session.passkeyMade()).toBe(true);
  });

  it('follows the session another tab of this browser keeps or forgets', async () => {
    const fake = new FakeSession().answer('server/discover', DISCOVERED);
    const sent = startingOut(fake);
    const session = TestBed.inject(SessionService);
    await session.start();
    expect(session.signedIn()).toBe(false);
    localStorage.setItem(TOKEN_KEY, JSON.stringify(SIGNED_IN));
    window.dispatchEvent(new StorageEvent('storage', { key: TOKEN_KEY }));
    expect(session.signedIn()).toBe(true);
    expect(sent.token()).toBe(SIGNED_IN.token);
    window.dispatchEvent(new StorageEvent('storage', { key: 'glave-theme' }));
    expect(session.signedIn()).toBe(true);
    localStorage.removeItem(TOKEN_KEY);
    window.dispatchEvent(new StorageEvent('storage', { key: TOKEN_KEY }));
    expect(session.signedIn()).toBe(false);
  });

  it('a session refused in one tab never forgets a newer one another tab began', async () => {
    localStorage.setItem(TOKEN_KEY, JSON.stringify(SIGNED_IN));
    const fake = new FakeSession()
      .answer('server/discover', DISCOVERED)
      .answer('project/list', new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'ended' }));
    const sent = startingOut(fake);
    const session = TestBed.inject(SessionService);
    await session.start();
    const newer = { token: 'glave_session_newer', server: FAKE_SERVER };
    // Another tab signed in again before this tab heard of it.
    localStorage.setItem(TOKEN_KEY, JSON.stringify(newer));
    await expect(session.call('project/list', {})).rejects.toBeInstanceOf(ApiError);
    expect(JSON.parse(localStorage.getItem(TOKEN_KEY) ?? 'null')).toEqual(newer);
    expect(session.signedIn()).toBe(true);
    expect(sent.token()).toBe(newer.token);
  });

  it('stays signed in after a reload, or in a new tab, with the session this browser kept', async () => {
    localStorage.setItem(TOKEN_KEY, JSON.stringify(SIGNED_IN));
    const fake = new FakeSession().answer('server/discover', DISCOVERED);
    const sent = startingOut(fake);
    const session = TestBed.inject(SessionService);
    await session.start();
    expect(session.signedIn()).toBe(true);
    expect(sent.token()).toBe(SIGNED_IN.token);
  });

  it('never presents a session to a server with another id, or none', async () => {
    for (const discovered of [{ name: 'glave', id: 'another-server' }, { name: 'glave' }]) {
      TestBed.resetTestingModule();
      localStorage.setItem(TOKEN_KEY, JSON.stringify(SIGNED_IN));
      const fake = new FakeSession().answer('server/discover', discovered);
      const sent = startingOut(fake);
      const session = TestBed.inject(SessionService);
      await session.start();
      expect(session.signedIn()).toBe(false);
      expect(sent.token()).toBe('');
    }
  });

  it('stays signed out when the server cannot be reached, or the code is spent', async () => {
    const away = new FakeSession().answer(
      'server/discover',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'away' }),
    );
    startingOut(away);
    await TestBed.inject(SessionService).start();
    expect(TestBed.inject(SessionService).signedIn()).toBe(false);

    TestBed.resetTestingModule();
    history.replaceState(null, '', '/#code=spent');
    const spent = new FakeSession()
      .answer('server/discover', DISCOVERED)
      .answer('signin/redeem', new ApiError({ code: ErrorCode.NO_SIGN_IN, message: 'used' }));
    startingOut(spent);
    const session = TestBed.inject(SessionService);
    await session.start();
    expect(session.signedIn()).toBe(false);
  });

  it('signs out with the server, and signs other browsers out', async () => {
    const fake = new FakeSession();
    TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
    const session = TestBed.inject(SessionService);
    await session.signOutOthers();
    expect(fake.paramsOf('signin/end_others')).toEqual([{}]);
    expect(session.signedIn()).toBe(true);
    await session.signOut();
    expect(fake.paramsOf('signin/end')).toEqual([{}]);
    expect(session.signedIn()).toBe(false);
  });

  it('is with a glave elsewhere when signed in with another token', async () => {
    TestBed.configureTestingModule({ providers: provideFakeSession(new FakeSession(), ELSEWHERE) });
    const session = TestBed.inject(SessionService);
    expect(session.signedIn()).toBe(true);
    expect(session.local()).toBe(false);
    await session.signOut();
    expect(session.local()).toBe(false);
  });
});
