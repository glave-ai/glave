import type { AttachedFile } from '@glave/client';

/**
 * The largest file the composer sends to be kept, in bytes: what a message carries on every
 * transport once the file is in base64, and what the server takes. The server says which
 * kinds it takes; a file past this would never reach it.
 */
export const MAX_SENT = 10 * 1024 * 1024;

/** The image types shown in a chip as themselves. */
const SHOWN = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp']);

/** A file added to a message being written, on its way to being kept or kept. */
export interface Attachment {
  readonly key: string;
  readonly name: string;
  /** Its media type, as the person's computer says; `application/octet-stream` when it says none. */
  readonly mediaType: string;
  readonly bytes: number;
  /** A URL of the image to show in its chip, for an image; `null` otherwise. */
  readonly preview: string | null;
  readonly state: 'sending' | 'kept' | 'refused';
  /** Its content's hash, once kept. */
  readonly sha256: string | null;
  /** Why it was refused, as the server said, once refused. */
  readonly reason: string | null;
}

/** `file`'s media type, or the one that says it is unknown. */
export function mediaTypeOf(file: { readonly type: string }): string {
  return file.type === '' ? 'application/octet-stream' : file.type;
}

/** Whether a file of `mediaType` is an image its chip shows. */
export function shown(mediaType: string): boolean {
  return SHOWN.has(mediaType);
}

/** `bytes` as a person in `locale` reads a size: `512 bytes`, `214 kB`, `3.4 MB`. */
export function sizeOf(bytes: number, locale: string): string {
  const unit = (value: number, name: string, digits: number): string =>
    new Intl.NumberFormat(locale, {
      style: 'unit',
      unit: name,
      unitDisplay: name === 'byte' ? 'long' : 'short',
      maximumFractionDigits: digits,
    }).format(value);
  if (bytes < 1024) {
    return unit(bytes, 'byte', 0);
  }
  if (bytes < 1024 * 1024) {
    return unit(Math.round(bytes / 1024), 'kilobyte', 0);
  }
  const megabytes = bytes / (1024 * 1024);
  return unit(megabytes, 'megabyte', megabytes < 10 ? 1 : 0);
}

/** `bytes` in base64, as `file/upload` takes them. */
export function base64Of(bytes: Uint8Array): string {
  // A chunk at a time, so no call takes more arguments than an engine allows.
  const CHUNK = 0x8000;
  let binary = '';
  for (let start = 0; start < bytes.length; start += CHUNK) {
    binary += String.fromCharCode(...bytes.subarray(start, start + CHUNK));
  }
  return btoa(binary);
}

/** The files kept, as `turn/start` names them, in the order they were added. */
export function attachedOf(attachments: readonly Attachment[]): readonly AttachedFile[] {
  return attachments.flatMap((attachment) =>
    attachment.state === 'kept' && attachment.sha256 !== null
      ? [{ sha256: attachment.sha256, name: attachment.name }]
      : [],
  );
}
