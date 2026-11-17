import { ApiError, ErrorCode, type AssetKind, type MadeFile } from '@glave/client';

import { kindName, madeCount, madeLine, madeRoom, whyNot } from './made';

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
    const event = (type: string, data: Record<string, unknown>) => ({
      glave: 1,
      id: type,
      thread: 't',
      seq: 1,
      at: '2026-11-16T12:00:00Z',
      type,
      v: 1,
      data,
    });
    const events = [
      event('turn.ended', { outcome: 'completed' }),
      event('turn.ended', { outcome: 'completed', made: [{ path: 'a.pdf' }] }),
      event('turn.ended', { outcome: 'completed', made_left: { kind: 'waits' } }),
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

  it('says why a file couldn’t be read in the server’s words', () => {
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    expect(whyNot(gone)).toBe('This file is no longer here.');
    expect(whyNot('nothing')).toBe('glave could not be reached. Try again.');
  });
});
