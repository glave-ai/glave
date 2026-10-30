import { place, teamName } from './organizations';

describe('organizations', () => {
  it('names their own Personal, and a team by an Owner of it', () => {
    expect(place({ id: 'o', people: 1, own: true, role: 'owner', current: true }, 'en')).toEqual({
      id: 'o',
      name: 'Personal',
      note: 'Just you',
      current: true,
      letter: 'P',
    });
    const acme = { id: 'a', people: 12, role: 'member' as const, owner: 'Dana Lee' };
    expect(place(acme, 'en')).toEqual({
      id: 'a',
      name: "Dana Lee's team",
      note: '12 people',
      current: false,
      letter: 'D',
    });
    expect(teamName({ id: 't', people: 2, role: 'member' })).toBe('A team');
  });
});
