import { forget, readStored, storageOf, writeStored } from './storage';

/** A storage that refuses every call, as some private windows do. */
const REFUSING = {
  getItem: () => {
    throw new Error('denied');
  },
  setItem: () => {
    throw new Error('denied');
  },
  removeItem: () => {
    throw new Error('denied');
  },
} as unknown as Storage;

describe('storage', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('finds the page’s storage, or none', () => {
    expect(storageOf(document, 'localStorage')).toBe(localStorage);
    expect(storageOf(document, 'sessionStorage')).toBe(sessionStorage);
    const throwing = {
      get defaultView(): Window {
        throw new Error('denied');
      },
    } as unknown as Document;
    expect(storageOf(throwing, 'localStorage')).toBeNull();
  });

  it('stores, reads and forgets JSON', () => {
    writeStored(localStorage, 'k', { open: true, width: 400 });
    expect(readStored(localStorage, 'k')).toEqual({ open: true, width: 400 });
    forget(localStorage, 'k');
    expect(readStored(localStorage, 'k')).toBeNull();
  });

  it('reads what is not JSON as its text, and does nothing without storage', () => {
    localStorage.setItem('k', 'dark');
    expect(readStored(localStorage, 'k')).toBe('dark');
    expect(readStored(null, 'k')).toBeNull();
    expect(() => {
      writeStored(null, 'k', 1);
      forget(null, 'k');
    }).not.toThrow();
  });

  it('survives a storage that refuses', () => {
    expect(readStored(REFUSING, 'k')).toBeNull();
    expect(() => {
      writeStored(REFUSING, 'k', 1);
      forget(REFUSING, 'k');
    }).not.toThrow();
  });
});
