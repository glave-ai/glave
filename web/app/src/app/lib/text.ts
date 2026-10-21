/** What a failure the server did not explain is told as. */
export function somethingWrong(): string {
  return $localize`:@@text.somethingWrong:Something went wrong. Try again in a moment.`;
}

/** What a failure to reach the server is told as. */
export function unreached(): string {
  return $localize`:@@text.unreached:glave could not be reached. Try again.`;
}

/**
 * What a part of the page whose code could not load says, once the person asks for it: the
 * page was being left, or glave went out of reach.
 */
export function unloaded(): string {
  return $localize`:@@text.unloaded:This part of glave could not load. Reload the page to try again.`;
}

/** What a failure to remove an account is told as, a model account or a connected one. */
export function removeFailed(): string {
  return $localize`:@@text.removeFailed:glave could not remove that account. Try again in a moment.`;
}

/**
 * `text` as a sentence: a capital first (in a script that has them), and a full stop last
 * unless it ends as a sentence does in any language; the plain failure when empty.
 */
export function sentence(text: string): string {
  const trimmed = text.trim();
  if (trimmed === '') {
    return somethingWrong();
  }
  const capital = `${trimmed.charAt(0).toUpperCase()}${trimmed.slice(1)}`;
  return /\p{Sentence_Terminal}$/u.test(capital) ? capital : `${capital}.`;
}

/** Whether `text` is longer than `chars` characters or `lines` lines, so it may fold away. */
export function isLong(text: string, chars: number, lines: number): boolean {
  return text.length > chars || text.split('\n').length > lines;
}
