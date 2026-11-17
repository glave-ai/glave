import {
  ApiError,
  type AssetKind,
  type FileMade,
  type MadeFile,
  type MadeLeft,
  type RawEvent,
  type TurnFiles,
} from '@glave/client';

import { sizeOf } from './attachments';
import { UnreadableFile } from './files';
import { sentence, unreached } from './text';

/**
 * The room a note's line keeps until the server's words fill it: a space that doesn't collapse,
 * so its line keeps its height.
 */
const NOTE_ROOM = '\u00a0';

/** A file a turn made, as its card is drawn, with what tells it from the others. */
export interface MadeCard {
  readonly key: string;
  readonly file: MadeFile;
}

/** What a turn shows of the files it made, after its reply. */
export interface MadeShown {
  /** Each file the server described, a card each. */
  readonly files: readonly MadeCard[];
  /** Each file its log names that the server hasn't described yet: a card's room, unseen. */
  readonly room: readonly MadeCard[];
  /** What they leave out, in the server's sentence, or a line's room for it; `null` when nothing. */
  readonly note: string | null;
  /** Whether `note` is only a line's room, unseen, until the server says what was left out. */
  readonly noteRoom: boolean;
}

/** A turn that made nothing, or whose files the server couldn't say. */
export const NO_MADE: MadeShown = { files: [], room: [], note: null, noteRoom: false };

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
 * The `seq` of the latest of `events` that says what a turn made: a turn's end that made a file
 * or left one out, or a later look-over (`turn.made`); `null` when none does. Each new one is a
 * reason to read the files again.
 */
export function lastMaking(events: readonly RawEvent[]): number | null {
  let last: number | null = null;
  for (const event of events) {
    const made: unknown = event.data['made'];
    const ended =
      event.type === 'turn.ended' &&
      ((Array.isArray(made) && made.length > 0) || event.data['made_left'] != null);
    if (ended || event.type === 'turn.made') {
      last = event.seq;
    }
  }
  return last;
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

/**
 * Why a file couldn't be read: the server's words, or the page's own when what it read back
 * wasn't whole; anything else, a browser's own words among them, as not reached.
 */
export function whyNot(error: unknown): string {
  return error instanceof ApiError || error instanceof UnreadableFile
    ? sentence(error.message)
    : unreached();
}

/** The name a file is saved as, from where a turn made it: `out/Trip budget.xlsx`'s last part. */
function fileName(path: string): string {
  return path.split('/').at(-1) ?? path;
}

/**
 * The cards a turn's end says it made, before the server has described them: each card's room,
 * kept unseen so nothing below moves when the real ones come in. Each is kept or not, and read
 * or not, as its card will be, so a card whose line will give a reason keeps that line's room.
 */
export function madeRoom(made: readonly FileMade[]): readonly MadeFile[] {
  return made.map((file) => {
    const blob = typeof file.kept === 'object' ? file.kept.blob : null;
    return {
      kind: 'other',
      name: fileName(file.path),
      file_name: fileName(file.path),
      media_type: '',
      bytes: file.bytes,
      sha256: blob?.sha256 ?? null,
      previews: false,
      kept: '\u00a0',
      unread: blob !== null && file.read == null ? '\u00a0' : null,
      sentence: '',
    };
  });
}

/**
 * `files` as cards, each with a key of its own: its file's name and hash, and how many before it
 * had the same, so two copies of one file are two cards.
 */
export function madeCards(files: readonly MadeFile[]): readonly MadeCard[] {
  const seen = new Map<string, number>();
  return files.map((file) => {
    const named = `${file.file_name}/${file.sha256 ?? ''}`;
    const before = seen.get(named) ?? 0;
    seen.set(named, before + 1);
    return { key: `${named}/${String(before)}`, file };
  });
}

/**
 * What a turn shows of the files it made: those the server described (`described`), then a
 * card's room for each file its log names (`named`) that it hasn't described yet, as a later
 * look-over's are until it does, and the server's note, or a line's room for it while the log
 * says something was left out (`left`) and the server hasn't said what. Once the server couldn't
 * say (`failed`), what it described stays, and no room is kept.
 */
export function madeShown(
  named: readonly FileMade[],
  left: boolean,
  described: TurnFiles | undefined,
  failed: boolean,
): MadeShown {
  const unseen = failed ? [] : named.slice(described?.files.length ?? 0);
  const note = described?.note ?? null;
  const noteRoom = note === null && left && !failed;
  return {
    files: madeCards(described?.files ?? []),
    room: madeCards(madeRoom(unseen)),
    note: noteRoom ? NOTE_ROOM : note,
    noteRoom,
  };
}
