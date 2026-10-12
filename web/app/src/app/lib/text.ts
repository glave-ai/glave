/** Whether `text` is longer than `chars` characters or `lines` lines, so it may fold away. */
export function isLong(text: string, chars: number, lines: number): boolean {
  return text.length > chars || text.split('\n').length > lines;
}
