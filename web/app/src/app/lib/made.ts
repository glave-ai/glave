import type { AssetKind, FileMade, MadeFile } from '@glave/client';

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
 * sheets or slides it has in the server's words (else its size), and where it is kept, in the
 * server's words: `Spreadsheet · 2 sheets · Kept with this chat`.
 */
export function madeLine(file: MadeFile, locale: string): string {
  const size = file.parts ?? sizeOf(file.bytes, locale);
  return [kindName(file.kind), size, file.kept].join(' · ');
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
