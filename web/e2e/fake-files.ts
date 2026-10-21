import { Refusal } from './fake-records';

/**
 * A file kept for a message, as the server answers `file/upload`: its hash made of its name, so
 * each file has its own; a video refused as the server refuses it.
 */
export function keptFile(params: Record<string, unknown>): unknown {
  const name = String(params['name']);
  const mediaType = String(params['media_type']);
  if (mediaType.startsWith('video/')) {
    return new Refusal(
      -32602,
      "glave can't use videos yet: add a screenshot or a few images instead",
    );
  }
  const hex = Array.from(name, (char) => char.charCodeAt(0).toString(16).padStart(2, '0')).join('');
  const sha256 = hex.padEnd(64, '0').slice(0, 64);
  const bytes = Math.floor((String(params['data']).length * 3) / 4);
  return { file: { sha256, name, media_type: mediaType, bytes } };
}
