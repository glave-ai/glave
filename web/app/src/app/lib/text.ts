/** What a failure the server did not explain is told as. */
export const SOMETHING_WRONG = 'Something went wrong. Try again in a moment.';

/** `text` as a sentence: a capital first, a full stop last; the plain failure when empty. */
export function sentence(text: string): string {
  const trimmed = text.trim();
  if (trimmed === '') {
    return SOMETHING_WRONG;
  }
  const capital = `${trimmed.charAt(0).toUpperCase()}${trimmed.slice(1)}`;
  return /[.!?]$/u.test(capital) ? capital : `${capital}.`;
}

/** Whether `text` is longer than `chars` characters or `lines` lines, so it may fold away. */
export function isLong(text: string, chars: number, lines: number): boolean {
  return text.length > chars || text.split('\n').length > lines;
}
