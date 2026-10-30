import { place } from './organizations';

describe('organizations', () => {
  it('lists each by the name the server gives it, with who is in it and its letter', () => {
    const own = { id: 'o', name: 'Personal', people: 1, own: true, role: 'owner', current: true };
    expect(place({ ...own, role: 'owner' as const }, 'en')).toEqual({
      id: 'o',
      name: 'Personal',
      note: 'Just you',
      current: true,
      letter: 'P',
    });
    const acme = { id: 'a', name: "dana@example.com's team", people: 12, role: 'member' as const };
    expect(place(acme, 'en')).toEqual({
      id: 'a',
      name: "dana@example.com's team",
      note: '12 people',
      current: false,
      letter: 'D',
    });
    expect(place({ ...acme, people: 1 }, 'en').note).toBe('1 person');
  });
});
