import type { InviteInfo, PersonInfo } from '@glave/client';

import { counted, inviteRow, isRole, lasts, personRow, roleName, untilIn } from './people';

/** A day as the page writes it, for these tests: the date alone. */
const day = (at: string): string => at.slice(0, 10);

const DANA: PersonInfo = {
  id: 'd',
  name: 'Dana Lee',
  email: 'dana@example.com',
  role: 'owner',
  you: true,
};
const SAM: PersonInfo = {
  id: 's',
  name: 'Sam Ortiz',
  role: 'admin',
  roles: ['admin', 'member'],
  removable: true,
};

describe('people', () => {
  it('names each role', () => {
    expect([roleName('owner'), roleName('admin'), roleName('member')]).toEqual([
      'Owner',
      'Admin',
      'Member',
    ]);
  });

  it('shows a person by name with their email, by email alone, or by who they are', () => {
    const dana = personRow(DANA, day);
    expect([dana.name, dana.detail, dana.role, dana.changeable]).toEqual([
      'Dana Lee',
      'dana@example.com',
      'Owner · you',
      false,
    ]);
    const sam = personRow(SAM, day);
    expect([sam.roles.map((role) => role.label), sam.changeable, sam.removable]).toEqual([
      ['Admin', 'Member'],
      true,
      true,
    ]);
    expect(sam.roleLabel).toBe('Role for Sam Ortiz');
    const emailed = personRow(
      { id: 'e', email: 'alex@example.com', role: 'member', joined: '2026-10-12T09:00:00.000Z' },
      day,
    );
    expect([emailed.name, emailed.detail]).toEqual(['alex@example.com', 'Joined 2026-10-12']);
    const linked = personRow({ id: 'k', role: 'member', joined: '2026-10-20T09:00:00.000Z' }, day);
    expect([linked.name, linked.detail]).toEqual([
      'Someone without a name or email',
      'Joined 2026-10-20',
    ]);
    expect(personRow({ id: 'l', role: 'owner', local: true }, day).name).toBe(
      'The person who runs this glave',
    );
    expect(personRow({ id: 'l', role: 'owner', local: true, you: true }, day).name).toBe('You');
  });

  it('says whom an invite waits for, who made it and when it ends', () => {
    const now = Date.parse('2026-10-29T12:00:00.000Z');
    const until = untilIn('en', now);
    const invite: InviteInfo = {
      id: 'i',
      role: 'member',
      by: { kind: 'person', id: 'd' },
      made: '2026-10-29T12:00:00.000Z',
      expires: '2026-11-04T12:00:00.000Z',
      email: 'alex@example.com',
    };
    expect(inviteRow(invite, [DANA, SAM], until)).toEqual({
      id: 'i',
      to: 'alex@example.com',
      detail: 'Member · invited by you · ends in 6 days',
    });
    const linked = { ...invite, email: null, by: { kind: 'person' as const, id: 's' } };
    expect(inviteRow(linked, [DANA, SAM], until)).toEqual({
      id: 'i',
      to: 'A link, no email',
      detail: 'Member · invited by Sam Ortiz · ends in 6 days',
    });
    const tomorrow = { ...invite, expires: '2026-10-30T12:00:00.000Z' };
    expect(inviteRow(tomorrow, [], until).detail).toBe('Member · ends tomorrow');
  });

  it('counts the people and the invites waiting', () => {
    expect(counted(1, 0, 'en')).toBe('1 person');
    expect(counted(4, 2, 'en')).toBe('4 people · 2 invites waiting');
    expect(counted(4, 1, 'en')).toBe('4 people · 1 invite waiting');
  });

  it('knows a role, and says how long an invite works', () => {
    expect([isRole('admin'), isRole('guest'), isRole(null)]).toEqual([true, false, false]);
    const invite: InviteInfo = {
      id: 'i',
      role: 'member',
      by: { kind: 'person', id: 'd' },
      made: '2026-10-29T12:00:00.000Z',
      expires: '2026-11-05T12:00:00.000Z',
    };
    expect(lasts(invite, 'en')).toBe('7 days');
    expect(lasts({ ...invite, expires: '2026-10-30T12:00:00.000Z' }, 'en')).toBe('1 day');
  });
});
