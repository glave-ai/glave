/**
 * This browser's storage, kept for small per-person conveniences (a panel's width, the theme)
 * and its half of the sign-in. It may be missing or refuse every call, as in some private
 * windows, so every helper here answers `null` or does nothing then, and the app works the same
 * without it.
 */
export type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null;

/** The page's local storage, or `null` when it has none it may use. */
export function storageOf(document: Document): Store {
  try {
    return document.defaultView?.localStorage ?? null;
  } catch {
    return null;
  }
}

/**
 * The value stored under `key`: its JSON, or the text itself when it is not JSON (the theme and
 * the token were once kept as plain text); `null` when there is none or it cannot be read.
 */
export function readStored(storage: Store, key: string): unknown {
  let text: string | null = null;
  try {
    text = storage?.getItem(key) ?? null;
    return text === null ? null : (JSON.parse(text) as unknown);
  } catch {
    return text;
  }
}

/** Stores `value` as JSON under `key`, if storage allows it; else it lasts for this page. */
export function writeStored(storage: Store, key: string, value: unknown): void {
  try {
    storage?.setItem(key, JSON.stringify(value));
  } catch {
    // Full or refused: nothing is kept.
  }
}

/** Forgets what is stored under `key`, if storage allows it. */
export function forget(storage: Store, key: string): void {
  try {
    storage?.removeItem(key);
  } catch {
    // Refused: nothing to forget that could be read back either.
  }
}
