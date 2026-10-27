import type { WorkspaceBound } from '@glave/client';

import { placeOf } from './place';

function bound(kind: WorkspaceBound['computer']['kind']): WorkspaceBound {
  return {
    workspace: '01900000-0000-7000-8000-0000000000d1',
    computer: { id: '01900000-0000-7000-8000-0000000000c1', kind },
    opened: { kind: 'folder', path: '/x' },
    root: '/x',
  };
}

describe('place', () => {
  it('names the computer the thread runs on, as its summary says, for the reader', () => {
    const local = { local: true, mac: false };
    expect(placeOf(bound('local'), { local: true, mac: true })).toBe('Your Mac');
    expect(placeOf(bound('local'), local)).toBe('This computer');
    expect(placeOf(bound('local'), { local: false, mac: true })).toBe('The server');
    expect(placeOf(bound('worker'), local)).toBe('Another computer');
    expect(placeOf(null, local)).toBeNull();
  });
});
