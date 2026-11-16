import type { AssetKind, LibraryItem, Project } from '@glave/client';

import { chatsName } from './nav';
import { threadTitle } from './threads';
import { calendarDay, type Reader } from './when';

/** What a filter takes when it keeps everything. */
export const EVERY = 'all';

/** What the person keeps in view: one kind or every kind, one project or every project. */
export interface LibraryFilter {
  readonly kind: string;
  readonly project: string;
}

/** The items `filter` keeps, in the server's order. */
export function filtered(
  items: readonly LibraryItem[],
  filter: LibraryFilter,
): readonly LibraryItem[] {
  return items.filter(
    (item) =>
      (filter.kind === EVERY || item.kind === filter.kind) &&
      (filter.project === EVERY || item.project === filter.project),
  );
}

/** One choice of a filter: its value and the words for it. */
export interface FilterChoice {
  readonly value: string;
  readonly label: string;
}

/** The kinds a person can filter by: every kind there is, each in the reader's words. */
export function kindChoices(): readonly FilterChoice[] {
  const kinds: readonly AssetKind[] = ['picture'];
  return [
    { value: EVERY, label: $localize`:@@library.everyKind:Everything` },
    ...kinds.map((kind) => ({ value: kind, label: kindLabel(kind) })),
  ];
}

/** A kind of item, in the reader's words, as a filter names it. */
export function kindLabel(kind: AssetKind): string {
  const labels: Readonly<Record<AssetKind, () => string>> = {
    picture: () => $localize`:@@library.pictures:Pictures`,
  };
  return labels[kind]();
}

/**
 * The projects a person can filter by: the ones their items were made in, in the order the
 * person's projects come (`projects`), each by its name, their Chats as the app calls them; one
 * no longer listed is left out.
 */
export function projectChoices(
  items: readonly LibraryItem[],
  projects: readonly Project[],
): readonly FilterChoice[] {
  const used = new Set(items.map((item) => item.project));
  return [
    { value: EVERY, label: $localize`:@@library.everyProject:Every project` },
    ...projects
      .filter((project) => used.has(project.id))
      .map((project) => ({
        value: project.id,
        label: project.kind === 'personal' ? chatsName() : project.name,
      })),
  ];
}

/** An item as the Library draws it: the server's facts, and the words and links made of them. */
export interface ShownItem {
  readonly item: LibraryItem;
  /** When it was made, in the reader's words. */
  readonly when: string;
  /** What opening it is called, for those who can't see the picture. */
  readonly openLabel: string;
  /** The chat it came from, while that chat is there. */
  readonly chat: { readonly link: string; readonly title: string } | null;
}

/** `item` as the Library draws it for `reader` at `now`. */
export function shown(item: LibraryItem, now: number, reader: Reader = {}): ShownItem {
  const name = item.name;
  const chat = item.chat ?? null;
  return {
    item,
    when: madeWhen(item.made_at, now, reader),
    openLabel: $localize`:@@library.open:Open ${name}:NAME:`,
    chat:
      chat === null
        ? null
        : {
            link: `/t/${chat.thread}`,
            title: threadTitle(chat),
          },
  };
}

/** How many days before `now` a moment is. */
const DAY = 24 * 60 * 60 * 1000;

/**
 * When an item was made, as the Library writes it for `reader` at `now` (milliseconds since the
 * epoch): today with its time, yesterday, a weekday within the week, else its date, with its
 * year when that is not this year's.
 */
export function madeWhen(at: string, now: number, reader: Reader = {}): string {
  const then = Date.parse(at);
  if (Number.isNaN(then)) {
    return '';
  }
  const { locale, timeZone } = reader;
  const date = (options: Intl.DateTimeFormatOptions): string =>
    new Intl.DateTimeFormat(locale, { timeZone, ...options }).format(then);
  const dayOf = calendarDay(timeZone);
  const day = dayOf(then);
  const today = dayOf(now);
  const before = Math.round((Date.parse(today) - Date.parse(day)) / DAY);
  if (before <= 0) {
    const time = date({ timeStyle: 'short' });
    return $localize`:@@library.today:Today, ${time}:TIME:`;
  }
  if (before === 1) {
    return $localize`:@@library.yesterday:Yesterday`;
  }
  if (before < 7) {
    return date({ weekday: 'long' });
  }
  return day.slice(0, 4) === today.slice(0, 4)
    ? date({ day: 'numeric', month: 'long' })
    : date({ dateStyle: 'long' });
}
