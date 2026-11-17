import { ApiError, ErrorCode, type AssetKind, type MadeFile, type RawEvent } from '@glave/client';

import {
  kindName,
  madeCount,
  madeLater,
  madeLeftOf,
  madeLine,
  madeOf,
  madeRoom,
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
    expect(madeLine({ ...DECK, kept: 'Saved to your Library' }, 'en')).toBe(
      'Slides · 8 slides · Saved to your Library',
    );
  });

  it('says why a file only downloads where the server couldn’t read it', () => {
    const unread = 'glave couldn’t read this file to show it: download it to open it';
    expect(madeLine({ ...DECK, parts: null, unread }, 'en')).toBe(`Slides · 3.3 MB · ${unread}`);
  });

  it('counts each time a log says what turns made, to read the files again', () => {
    const events = [
      event('turn.ended', { outcome: 'completed' }),
      event('turn.ended', { outcome: 'completed', made: [{ path: 'a.pdf' }] }),
      event('turn.ended', { outcome: 'completed', made_left: { kind: 'more', files: 1 } }),
      event('turn.made', { turn: 3 }),
      event('message.added', {}),
    ];
    expect(madeCount(events)).toBe(3);
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
    expect(whyNot('nothing')).toBe('glave could not be reached. Try again.');
  });
});
