import type { AssetKind, FileMade, MadeFile, MadeLeft, RawEvent } from '@glave/client';

import { sizeOf } from './attachments';
import { sentence, unreached } from './text';

/** What a file of `kind` is called, for a person. */
export function kindName(kind: AssetKind): string {
  switch (kind) {
    case 'document':
      return $localize`:@@made.document:Document`;
    case 'spreadsheet':
      return $localize`:@@made.spreadsheet:Spreadsheet`;
    case 'deck':
      return $localize`:@@made.deck:Slides`;
    case 'pdf':
      return $localize`:@@made.pdf:PDF`;
    case 'picture':
      return $localize`:@@made.picture:Picture`;
    case 'other':
      return $localize`:@@made.other:File`;
  }
}

/**
 * The line under a file's name on its card, for a reader in `locale`: its kind, how many pages,
 * sheets or slides it has in the server's words (else its size), and where it is kept, or why it
 * only downloads, in the server's words: `Spreadsheet · 2 sheets · Kept with this chat`.
 */
export function madeLine(file: MadeFile, locale: string): string {
  const size = file.parts ?? sizeOf(file.bytes, locale);
  return [kindName(file.kind), size, file.unread ?? file.kept].join(' · ');
}

/**
 * How many times `events` say what turns made: each turn's end that made a file or left one out,
 * and each later look-over. Each new one is a reason to read the files again.
 */
export function madeCount(events: readonly RawEvent[]): number {
  return events.filter((event) => {
    if (event.type === 'turn.made') {
      return true;
    }
    if (event.type !== 'turn.ended') {
      return false;
    }
    const made: unknown = event.data['made'];
    return (Array.isArray(made) && made.length > 0) || event.data['made_left'] != null;
  }).length;
}

/**
 * The files a turn's end or a later look-over says it made: each its path and size, which keep
 * its card's room until the server describes it; one that is not a file it made is left out.
 */
export function madeOf(value: unknown): readonly FileMade[] {
  const list = Array.isArray(value) ? (value as readonly unknown[]) : [];
  return list.filter(
    (each): each is FileMade =>
      typeof each === 'object' &&
      each !== null &&
      typeof (each as Partial<FileMade>).path === 'string' &&
      typeof (each as Partial<FileMade>).bytes === 'number',
  );
}

/** What a turn's end or a later look-over says its files leave out; `null` when nothing. */
export function madeLeftOf(value: unknown): MadeLeft | null {
  if (typeof value !== 'object' || value === null) {
    return null;
  }
  const { kind, files } = value as { readonly kind?: unknown; readonly files?: unknown };
  if (kind === 'too_many') {
    return { kind };
  }
  return kind === 'more' && typeof files === 'number' ? { kind, files } : null;
}

/** What a turn's later look-overs said it made, before the server describes it. */
export interface MadeLater {
  readonly files: readonly FileMade[];
  /** Whether they leave something out, which a note under the cards will say. */
  readonly left: boolean;
}

/** What later look-overs (`turn.made`) in `events` said each turn made, by its start's `seq`. */
export function madeLater(events: readonly RawEvent[]): ReadonlyMap<number, MadeLater> {
  const later = new Map<number, MadeLater>();
  for (const event of events) {
    const turn = event.data['turn'];
    if (event.type !== 'turn.made' || typeof turn !== 'number') {
      continue;
    }
    const was = later.get(turn) ?? { files: [], left: false };
    later.set(turn, {
      files: [...was.files, ...madeOf(event.data['made'])],
      left: was.left || madeLeftOf(event.data['made_left']) !== null,
    });
  }
  return later;
}

/** Why a file couldn't be read, from what `error` says: the server's words, or unreached. */
export function whyNot(error: unknown): string {
  return error instanceof Error ? sentence(error.message) : unreached();
}

/** The name a file is saved as, from where a turn made it: `out/Trip budget.xlsx`'s last part. */
function fileName(path: string): string {
  return path.split('/').at(-1) ?? path;
}

/**
 * The cards a turn's end says it made, before the server has described them: each card's room,
 * kept unseen so nothing below moves when the real ones come in.
 */
export function madeRoom(made: readonly FileMade[]): readonly MadeFile[] {
  return made.map((file) => ({
    kind: 'other',
    name: fileName(file.path),
    file_name: fileName(file.path),
    media_type: '',
    bytes: file.bytes,
    previews: false,
    kept: ' ',
    sentence: '',
  }));
}
