import type { RawEvent } from '@glave/client';

import { type Answers, Refusal, bound, call, event } from './fake-records';

/**
 * A chat whose agent read two photos: each call's result holds its image, which the page draws
 * under the step, read back through `file/read`. The server still holds the first; the second
 * is no longer here.
 */
export const PHOTOS = '01900000-0000-7000-8000-000000000161';

/** A one-pixel PNG, in base64. */
const PIXEL =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';

/** The photos, as the thread holds them. */
const CAT = { sha256: 'ca'.repeat(32), media_type: 'image/png', bytes: 70 };
const DOG = { sha256: 'd0'.repeat(32), media_type: 'image/png', bytes: 70 };

/** The `read` of the photo `name`, kept as `blob`: its call and its result, from `seq`. */
function read(seq: number, name: string, blob: typeof CAT): readonly RawEvent[] {
  const id = `call_${String(seq)}`;
  return [
    call(seq, id, 'read', { path: name }, PHOTOS),
    event(
      seq + 1,
      'tool.call.completed',
      {
        call_id: id,
        is_error: false,
        output: [
          { type: 'text', text: `${name} is an image of 70 bytes.` },
          { type: 'file', blob, name },
        ],
      },
      PHOTOS,
    ),
  ];
}

/** Its history: the photos read, then what the agent saw in them. */
export const PHOTOS_HISTORY = [
  event(1, 'thread.started', {}, PHOTOS),
  bound(2, PHOTOS),
  event(3, 'turn.started', {}, PHOTOS),
  event(
    4,
    'message.added',
    { role: 'user', parts: [{ type: 'text', text: 'What is in cat.png and dog.png?' }] },
    PHOTOS,
  ),
  ...read(5, 'cat.png', CAT),
  ...read(7, 'dog.png', DOG),
  event(
    9,
    'message.added',
    { role: 'assistant', parts: [{ type: 'text', text: 'A cat asleep, and a dog awake.' }] },
    PHOTOS,
  ),
  event(10, 'turn.ended', { outcome: 'completed' }, PHOTOS, '2026-10-11T10:00:05.000Z'),
];

/** The cat, kept in the person's Library from the chat, in their Chats (fake-api.ts's PERSONAL). */
const KEPT = {
  asset: '01900000-0000-7000-8000-000000000171',
  kind: 'picture',
  name: 'A cat on the windowsill',
  file_name: 'cat.png',
  media_type: CAT.media_type,
  bytes: CAT.bytes,
  project: '01900000-0000-7000-8000-000000000002',
  made_at: '2026-10-11T10:00:04.000Z',
  chat: { thread: PHOTOS, title: 'What are these?' },
} as const;

/**
 * Its photos, as `file/read` gives them back, and the first, kept in the person's Library, as
 * `library/list` and `library/read` give it. No list holds the chat, and `thread/get` tells it
 * as any thread the fake does not name.
 */
export const ABOUT_PHOTOS: Answers<null> = {
  'library/list': () => ({ items: [KEPT] }),
  'library/read': (params) =>
    params.asset === KEPT.asset
      ? { piece: { media_type: CAT.media_type, bytes: CAT.bytes, offset: 0, data: PIXEL } }
      : new Refusal(-32030, 'this file is no longer here'),
  'file/read': (params) => {
    if (params.thread !== PHOTOS) {
      return undefined;
    }
    return params.sha256 === CAT.sha256
      ? { piece: { media_type: CAT.media_type, bytes: CAT.bytes, offset: 0, data: PIXEL } }
      : new Refusal(-32030, 'this file is no longer here');
  },
};
