import { localStorageOf, readStored, writeStored } from './storage';

/** A storage that refuses every call, as some private windows do. */
const REFUSING = {
  getItem: () => {
    throw new Error('denied');
  },
  setItem: () => {
    throw new Error('denied');
  },
} as unknown as Storage;

describe('storage', () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it('finds the page’s local storage, or none', () => {
    expect(localStorageOf(document)).toBe(localStorage);
    const throwing = {
      get defaultView(): Window {
        throw new Error('denied');
      },
    } as unknown as Document;
    expect(localStorageOf(throwing)).toBeNull();
  });

  it('stores and reads JSON', () => {
    writeStored(localStorage, 'k', { open: true, width: 400 });
    expect(readStored(localStorage, 'k')).toEqual({ open: true, width: 400 });
    expect(readStored(localStorage, 'missing')).toBeNull();
  });

  it('answers null for what does not parse, and does nothing without storage', () => {
    localStorage.setItem('k', '{not json');
    expect(readStored(localStorage, 'k')).toBeNull();
    expect(readStored(null, 'k')).toBeNull();
    expect(() => {
      writeStored(null, 'k', 1);
    }).not.toThrow();
  });

  it('survives a storage that refuses', () => {
    expect(readStored(REFUSING, 'k')).toBeNull();
    expect(() => {
      writeStored(REFUSING, 'k', 1);
    }).not.toThrow();
  });
});
