import { TOKEN_KEY, forgetToken, takeToken } from './token';

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

describe('token', () => {
  it('takes the token from the link, out of the address bar and into the tab', () => {
    const kept = storage();
    const replaceState = vi.fn();
    const token = takeToken({
      location: { hash: '#token=glave_local_abc', pathname: '/t/1', search: '?x=1' },
      history: { replaceState },
      storage: kept,
    });
    expect(token).toBe('glave_local_abc');
    expect(replaceState).toHaveBeenCalledWith(null, '', '/t/1?x=1');
    expect(kept.kept.get(TOKEN_KEY)).toBe('"glave_local_abc"');
  });

  it('reads a kept token when the link has none, and forgets it', () => {
    const kept = storage();
    kept.setItem(TOKEN_KEY, 'kept');
    const replaceState = vi.fn();
    const place = { location: { hash: '', pathname: '/', search: '' }, history: { replaceState } };
    expect(takeToken({ ...place, storage: kept })).toBe('kept');
    expect(replaceState).not.toHaveBeenCalled();
    forgetToken(kept);
    expect(takeToken({ ...place, storage: kept })).toBeNull();
    expect(takeToken({ ...place, storage: null })).toBeNull();
  });
});
