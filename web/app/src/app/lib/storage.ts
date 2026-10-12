/**
 * This browser's local storage, kept for small per-person conveniences (a panel's width).
 * It may be missing or refuse every call, as in some private windows, so every helper here
 * answers `null` or does nothing then, and the app works the same without it.
 */
export function localStorageOf(document: Document): Storage | null {
  try {
    return document.defaultView?.localStorage ?? null;
  } catch {
    return null;
  }
}

/** The JSON value stored under `key`, or `null` when there is none or it cannot be read. */
export function readStored(storage: Storage | null, key: string): unknown {
  try {
    const raw = storage?.getItem(key) ?? null;
    return raw === null ? null : (JSON.parse(raw) as unknown);
  } catch {
    return null;
  }
}

/** Stores `value` as JSON under `key`, if storage allows it. */
export function writeStored(storage: Storage | null, key: string, value: unknown): void {
  try {
    storage?.setItem(key, JSON.stringify(value));
  } catch {
    // Full or refused: the value lasts for this page only.
  }
}
