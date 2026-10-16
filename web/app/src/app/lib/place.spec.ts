import { event } from '../testing/fixtures';
import { placeOf } from './place';

function bound(kind: string): ReturnType<typeof event> {
  return event(2, 'workspace.bound', { computer: { id: 'c', kind }, root: '/x' });
}

describe('place', () => {
  it('says where the thread runs, as far as the app knows', () => {
    const local = { local: true, mac: false };
    expect(placeOf([bound('local')], { local: true, mac: true })).toBe('on your Mac');
    expect(placeOf([bound('local')], local)).toBe('on this computer');
    expect(placeOf([bound('local')], { local: false, mac: true })).toBe('on the server');
    expect(placeOf([bound('worker')], local)).toBe('on another computer');
    expect(placeOf([], local)).toBeNull();
    expect(placeOf([event(2, 'workspace.bound', {})], local)).toBeNull();
  });
});
