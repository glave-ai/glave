import { type Attachment, attachedOf, base64Of, mediaTypeOf, shown, sizeOf } from './attachments';

function attachment(more: Partial<Attachment>): Attachment {
  return {
    key: 'a',
    name: 'notes.txt',
    mediaType: 'text/plain',
    bytes: 3,
    preview: null,
    state: 'kept',
    sha256: 'f'.repeat(64),
    reason: null,
    ...more,
  };
}

describe('attachments', () => {
  it('sizes files as a person reads them', () => {
    expect(sizeOf(512)).toBe('512 B');
    expect(sizeOf(214 * 1024)).toBe('214 KB');
    expect(sizeOf(3.4 * 1024 * 1024)).toBe('3.4 MB');
    expect(sizeOf(12 * 1024 * 1024)).toBe('12 MB');
  });

  it('writes bytes in base64, past one chunk too', () => {
    expect(base64Of(new TextEncoder().encode('hi there'))).toBe('aGkgdGhlcmU=');
    const long = new Uint8Array(0x8000 * 2 + 1).fill(65);
    expect(atob(base64Of(long))).toBe('A'.repeat(long.length));
  });

  it('knows an unknown type and the images it shows', () => {
    expect(mediaTypeOf({ type: '' })).toBe('application/octet-stream');
    expect(mediaTypeOf({ type: 'image/png' })).toBe('image/png');
    expect(shown('image/webp')).toBe(true);
    expect(shown('image/svg+xml')).toBe(false);
  });

  it('names only the files kept, in order', () => {
    const kept = attachment({ key: 'b', name: 'cat.png', sha256: 'a'.repeat(64) });
    expect(
      attachedOf([
        attachment({ state: 'sending', sha256: null }),
        kept,
        attachment({ key: 'c', state: 'refused', sha256: null, reason: 'too large' }),
      ]),
    ).toEqual([{ sha256: 'a'.repeat(64), name: 'cat.png' }]);
  });
});
