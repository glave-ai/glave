import {
  ApiError,
  ErrorCode,
  type AssetKind,
  type FileMade,
  type MadeFile,
  type RawEvent,
} from '@glave/client';

import { UnreadableFile } from './files';
import {
  kindName,
  lastMaking,
  byPath,
  madeCards,
  madeLater,
  madeLeftOf,
  madeLine,
  madeOf,
  madeRoom,
  madeShown,
  whyNot,
} from './made';

/** An event of `type` holding `data`, at `seq`. */
function event(type: string, data: Record<string, unknown>, seq = 1): RawEvent {
  return { glave: 1, id: type, thread: 't', seq, at: '2026-11-16T12:00:00Z', type, v: 1, data };
}

const DECK: MadeFile = {
  kind: 'deck',
  name: 'Pitch to the group',
  file_name: 'pitch.pptx',
  media_type: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  bytes: 3_500_000,
  sha256: 'd'.repeat(64),
  parts: '8 slides',
  previews: true,
  kept: 'Kept with this chat',
  sentence: 'Made a deck: Pitch to the group (8 slides)',
};

describe('made', () => {
  it('names each kind of file for a person', () => {
    const kinds: readonly AssetKind[] = ['document', 'spreadsheet', 'deck', 'pdf', 'other'];
    expect(kinds.map(kindName)).toEqual(['Document', 'Spreadsheet', 'Slides', 'PDF', 'File']);
  });

  it('says a file’s kind, its parts in the server’s words, and where the server keeps it', () => {
    expect(madeLine(DECK, 'en')).toBe('Slides · 8 slides · Kept with this chat');
    // Who it is shared with stands where it is kept.
    expect(madeLine({ ...DECK, shared: 'Shared with 2 people' }, 'en')).toBe(
      'Slides · 8 slides · Shared with 2 people',
    );
    expect(madeLine({ ...DECK, kept: 'Saved to your Library' }, 'en')).toBe(
      'Slides · 8 slides · Saved to your Library',
    );
  });

  it('says why a file only downloads where the server couldn’t read it', () => {
    const unread = 'glave couldn’t read this file to show it: download it to open it';
    expect(madeLine({ ...DECK, parts: null, unread }, 'en')).toBe(`Slides · 3.3 MB · ${unread}`);
  });

  it('reads the files again on what last said what turns made, and on nothing else', () => {
    const ended = event('turn.ended', { outcome: 'completed', made: [{ path: 'a.pdf' }] }, 2);
    const left = event('turn.ended', { made_left: { kind: 'more', files: 1 } }, 3);
    expect(lastMaking([event('turn.ended', { outcome: 'completed', made: [] }, 1)])).toBeNull();
    expect(lastMaking([ended, event('message.added', {}, 5)])).toBe(2);
    expect(lastMaking([ended, left, event('turn.ended', { outcome: 'completed' }, 4)])).toBe(3);
    expect(lastMaking([ended, event('turn.made', { turn: 1 }, 6)])).toBe(6);
  });

  it('says its size where the server knows no parts', () => {
    expect(madeLine({ ...DECK, parts: null }, 'en')).toBe('Slides · 3.3 MB · Kept with this chat');
  });

  it('keeps a card’s room for each file a turn’s end names, before the server describes it', () => {
    const room = madeRoom([
      { path: 'out/Trip budget.xlsx', bytes: 12, kept: 'too_large' },
      { path: 'plan.pdf', bytes: 9, kept: 'secret' },
    ]);
    expect(room.map((file) => file.file_name)).toEqual(['Trip budget.xlsx', 'plan.pdf']);
    expect(room.every((file) => !file.previews && file.sha256 == null)).toBe(true);
    // Kept and read, or kept and not read, as its card will be, so its line keeps its room.
    const blob = { sha256: 'b'.repeat(64), bytes: 9, media_type: 'application/pdf' };
    const read = { parts: { pages: 1 } };
    const [shown, unread] = madeRoom([
      { path: 'plan.pdf', bytes: 9, kept: { blob }, read },
      { path: 'deck.pptx', bytes: 9, kept: { blob } },
    ]);
    expect([shown?.sha256, shown?.unread]).toEqual([blob.sha256, null]);
    expect([unread?.sha256, unread?.unread]).toEqual([blob.sha256, '\u00a0']);
  });

  it('tells each path once, a later look-over’s in the place of the first', () => {
    const first: FileMade = { path: 'plan.pdf', bytes: 9, kept: 'too_large' };
    const deck: FileMade = { path: 'deck.pptx', bytes: 4, kept: 'secret' };
    const again: FileMade = { path: 'plan.pdf', bytes: 12, kept: 'secret' };
    expect(byPath([first, deck, again])).toEqual([again, deck]);
  });

  it('keys each card by its file, two copies of one file apart', () => {
    const keys = madeCards([DECK, DECK, { ...DECK, sha256: null }]).map((card) => card.key);
    expect(new Set(keys).size).toBe(3);
  });

  it('shows what the server described, the room of what it hasn’t yet, and the note’s room', () => {
    const plan: FileMade = { path: 'plan.pdf', bytes: 9, kept: 'too_large' };
    const deck: FileMade = { path: 'deck.pptx', bytes: 4, kept: 'secret' };
    const files = (shown: ReturnType<typeof madeShown>): readonly string[] =>
      [...shown.files, ...shown.room].map((card) => card.file.file_name);
    // Before the server describes them, a card's room for each, and the note's line.
    const before = madeShown([plan], true, undefined, false);
    expect([files(before), before.room.length, before.note, before.noteRoom]).toEqual([
      ['plan.pdf'],
      1,
      '\u00a0',
      true,
    ]);
    // Described, then a later look-over names one more: its room after the cards drawn.
    const described = { turn: 1, files: [{ ...DECK, file_name: 'plan.pdf' }], note: 'One more' };
    const later = madeShown([plan, deck], true, described, false);
    expect([files(later), later.files.length, later.note, later.noteRoom]).toEqual([
      ['plan.pdf', 'deck.pptx'],
      1,
      'One more',
      false,
    ]);
    // A read that fails after one that didn't keeps the cards drawn, and no room.
    const failed = madeShown([plan, deck], true, described, true);
    expect([files(failed), failed.note]).toEqual([['plan.pdf'], 'One more']);
    expect(madeShown([plan], true, undefined, true).room).toEqual([]);
  });

  it('reads the files a log names, and what they leave out, leaving out what is not one', () => {
    expect(
      madeOf([{ path: 'plan.pdf', bytes: 9, kept: 'too_large' }, { path: 7 }, 'a.pdf', null]),
    ).toEqual([{ path: 'plan.pdf', bytes: 9, kept: 'too_large' }]);
    expect(madeOf('plan.pdf')).toEqual([]);
    expect(madeLeftOf({ kind: 'more', files: 2 })).toEqual({ kind: 'more', files: 2 });
    expect(madeLeftOf({ kind: 'too_many' })).toEqual({ kind: 'too_many' });
    expect(madeLeftOf({ kind: 'more' })).toBeNull();
    expect(madeLeftOf({ kind: 'waits' })).toBeNull();
    expect(madeLeftOf(null)).toBeNull();
  });

  it('gathers what later look-overs said each turn made, by its start', () => {
    const plan = { path: 'plan.pdf', bytes: 9, kept: 'too_large' };
    const deck = { path: 'deck.pptx', bytes: 4, kept: 'secret' };
    const later = madeLater([
      event('turn.made', { turn: 3, made: [plan] }, 20),
      event('turn.ended', { outcome: 'completed', made: [deck] }, 21),
      event('turn.made', { turn: 3, made: [deck], made_left: { kind: 'more', files: 1 } }, 22),
      event('turn.made', { turn: 9 }, 23),
      event('turn.made', { made: [plan] }, 24),
    ]);
    expect([...later.keys()]).toEqual([3, 9]);
    expect(later.get(3)).toEqual({ files: [plan, deck], left: true });
    expect(later.get(9)).toEqual({ files: [], left: false });
  });

  it('says why a file couldn’t be read in the server’s words', () => {
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    expect(whyNot(gone)).toBe('This file is no longer here.');
    expect(whyNot(new UnreadableFile('This file could not be read'))).toBe(
      'This file could not be read.',
    );
    // A browser's own words never reach the card.
    const unreached = 'glave could not be reached. Try again.';
    expect(whyNot(new TypeError('Failed to fetch'))).toBe(unreached);
    expect(whyNot(new DOMException('The operation was aborted.', 'AbortError'))).toBe(unreached);
    expect(whyNot('nothing')).toBe(unreached);
  });
});
