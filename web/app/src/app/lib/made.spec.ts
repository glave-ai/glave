import { ApiError, ErrorCode, type FileKind, type MadeFile } from '@glave/client';

import { kindName, madeLine, shownBy, whyNot } from './made';

const DECK: MadeFile = {
  seq: 4,
  sha256: 'd'.repeat(64),
  name: 'pitch.pptx',
  title: 'pitch',
  kind: 'deck',
  media_type: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  bytes: 3_500_000,
  kept: 'Kept with this chat',
};

describe('made', () => {
  it('names each kind of file for a person', () => {
    const kinds: readonly FileKind[] = ['document', 'spreadsheet', 'deck', 'pdf', 'other'];
    expect(kinds.map(kindName)).toEqual(['Document', 'Spreadsheet', 'Slides', 'PDF', 'File']);
  });

  it('says a file’s kind, size and where the server says it is kept, in a line', () => {
    expect(madeLine(DECK, 'en')).toBe('Slides · 3.3 MB · Kept with this chat');
    expect(madeLine({ ...DECK, kept: 'Saved to your Library' }, 'en')).toBe(
      'Slides · 3.3 MB · Saved to your Library',
    );
  });

  it('leaves a PDF to the browser and the rest to the server', () => {
    expect(shownBy({ ...DECK, kind: 'pdf' })).toBe('browser');
    expect(shownBy(DECK)).toBe('server');
  });

  it('says why a file couldn’t be read in the server’s words', () => {
    const gone = new ApiError({ code: ErrorCode.NO_FILE, message: 'this file is no longer here' });
    expect(whyNot(gone)).toBe('This file is no longer here.');
    expect(whyNot('nothing')).toBe('glave could not be reached. Try again.');
  });
});
