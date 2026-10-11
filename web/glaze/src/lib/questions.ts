/** One choice offered with a question. */
export interface GlzChoice {
  readonly label: string;
  readonly description?: string | null;
}

/** A question the model asks, with optional choices. */
export interface GlzQuestion {
  readonly question: string;
  readonly options?: readonly GlzChoice[];
  /** Whether more than one choice may be picked. */
  readonly multiple?: boolean;
}

/** An answer as one string: the choices picked, then the person's own words, if any. */
export function composeAnswer(picked: readonly string[], ownWords: string): string {
  return [...picked, ownWords.trim()].filter((part) => part !== '').join(', ');
}

/** `picked` with `label` added or taken out. */
export function togglePick(
  picked: readonly string[],
  label: string,
  on: boolean,
): readonly string[] {
  const without = picked.filter((item) => item !== label);
  return on ? [...without, label] : without;
}

/** A copy of `list` with `index` set to `value`. */
export function replaceAt<T>(list: readonly T[], index: number, value: T): readonly T[] {
  return list.map((item, at) => (at === index ? value : item));
}
