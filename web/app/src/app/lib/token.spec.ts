import { TOKEN_KEY, isLocalToken, keepSession, readSession, sessionFor, takeCode } from './token';

function storage(): Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> & {
  readonly kept: Map<string, string>;
} {
  const kept = new Map<string, string>();
  return {
    kept,
    getItem: (key) => kept.get(key) ?? null,
    setItem: (key, value) => kept.set(key, value),
    removeItem: (key) => kept.delete(key),
  };
}

const SESSION = { token: 'glave_session_abc', server: 'server-1' } as const;

describe('token', () => {
  it('takes the code from the link, out of the address bar', () => {
    const replaceState = vi.fn();
    const code = takeCode(
      {
        location: { hash: '#code=abc', pathname: '/t/1', search: '?x=1' },
        history: { replaceState },
      },
      'code',
    );
    expect(code).toBe('abc');
    expect(replaceState).toHaveBeenCalledWith(null, '', '/t/1?x=1');
  });

  it('finds no code in a link without one, and leaves the address alone', () => {
    const replaceState = vi.fn();
    const place = { location: { hash: '', pathname: '/', search: '' }, history: { replaceState } };
    expect(takeCode(place, 'code')).toBeNull();
    const other = { ...place, location: { ...place.location, hash: '#token=x' } };
    expect(takeCode(other, 'code')).toBeNull();
    expect(replaceState).not.toHaveBeenCalled();
  });

  it("takes a setup link's code out of the address bar, and nothing else", () => {
    const replaceState = vi.fn();
    const at = (hash: string): Parameters<typeof takeCode>[0] => ({
      location: { hash, pathname: '/', search: '' },
      history: { replaceState },
    });
    expect(takeCode(at('#code=abc'), 'setup')).toBeNull();
    expect(takeCode(at('#setup='), 'setup')).toBeNull();
    expect(replaceState).not.toHaveBeenCalled();
    expect(takeCode(at('#setup=f00d'), 'setup')).toBe('f00d');
    expect(replaceState).toHaveBeenCalledWith(null, '', '/');
  });

  it('keeps a session for the tab, and reads back only a whole one', () => {
    const kept = storage();
    expect(readSession(kept)).toBeNull();
    keepSession(kept, SESSION);
    expect(readSession(kept)).toEqual(SESSION);
    kept.setItem(TOKEN_KEY, '"glave_local_old"');
    expect(readSession(kept)).toBeNull();
    expect(readSession(null)).toBeNull();
  });

  it('presents a session only to the server that made it', () => {
    expect(sessionFor(SESSION, 'server-1')).toEqual(SESSION);
    expect(sessionFor(SESSION, 'server-2')).toBeNull();
    expect(sessionFor(SESSION, undefined)).toBeNull();
    expect(sessionFor(null, 'server-1')).toBeNull();
  });

  it('knows a browser session, which only a glave on this computer gives', () => {
    expect(isLocalToken('glave_session_abc')).toBe(true);
    expect(isLocalToken('eyJhbGciOi.an-access-token')).toBe(false);
    expect(isLocalToken('')).toBe(false);
  });
});
