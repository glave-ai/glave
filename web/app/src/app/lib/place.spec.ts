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
  it('says where the thread runs, as its summary says, in words for the reader', () => {
    const local = { local: true, mac: false };
    expect(placeOf(bound('local'), { local: true, mac: true })).toBe('on your Mac');
    expect(placeOf(bound('local'), local)).toBe('on this computer');
    expect(placeOf(bound('local'), { local: false, mac: true })).toBe('on the server');
    expect(placeOf(bound('worker'), local)).toBe('on another computer');
    expect(placeOf(null, local)).toBeNull();
  });
});
