import type { FileKind, MadeFile } from '@glave/client';

import { sizeOf } from './attachments';
import { sentence, unreached } from './text';

/** What a file of `kind` is called, for a person. */
export function kindName(kind: FileKind): string {
  switch (kind) {
    case 'document':
      return $localize`:@@made.document:Document`;
    case 'spreadsheet':
      return $localize`:@@made.spreadsheet:Spreadsheet`;
    case 'deck':
      return $localize`:@@made.deck:Slides`;
    case 'pdf':
      return $localize`:@@made.pdf:PDF`;
    case 'other':
      return $localize`:@@made.other:File`;
  }
}

/**
 * The line under a file's title on its card, for a reader in `locale`: its kind, its size, and
 * where it is kept, in the server's words: `Spreadsheet · 12 kB · Kept with this chat`.
 */
export function madeLine(file: MadeFile, locale: string): string {
  return [kindName(file.kind), sizeOf(file.bytes, locale), file.kept].join(' · ');
}

/** Why a file couldn't be read, from what `error` says: the server's words, or unreached. */
export function whyNot(error: unknown): string {
  return error instanceof Error ? sentence(error.message) : unreached();
}

/** How a file the thread made is shown when opened: drawn by the browser, or by the server. */
export type Shown = 'browser' | 'server';

/** Whether the browser draws `file` (a PDF), or the server previews it as Markdown. */
export function shownBy(file: MadeFile): Shown {
  return file.kind === 'pdf' ? 'browser' : 'server';
}
