/**
 * A new idempotency key for a send, made as the person sends it: 128 random bits, as hex. From
 * `crypto.getRandomValues`, which a page served over plain HTTP has too, where `randomUUID`
 * needs a secure context that a person's own server, reached at its address, may not give.
 */
export function idempotencyKey(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}
