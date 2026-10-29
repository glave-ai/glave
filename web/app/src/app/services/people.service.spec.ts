import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type InviteInfo, type PersonInfo } from '@glave/client';

import { FakeSession, provideFakeSession } from '../testing/fake-session';
import { PeopleService } from './people.service';

const SAM: PersonInfo = { id: 's', name: 'Sam Ortiz', role: 'admin', removable: true };
const INVITE: InviteInfo = {
  id: 'i',
  role: 'member',
  by: { kind: 'person', id: 's' },
  made: '2026-10-29T12:00:00.000Z',
  expires: '2026-11-05T12:00:00.000Z',
};

function service(fake: FakeSession): PeopleService {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  return TestBed.inject(PeopleService);
}

describe('PeopleService', () => {
  it('reads the people and invites, and again after each change', async () => {
    let people: readonly PersonInfo[] = [SAM];
    const fake = new FakeSession()
      .answer('people/list', () => ({ people }))
      .answer('invite/list', { invites: [INVITE] })
      .answer('role/set', () => {
        people = [{ ...SAM, role: 'member' }];
        return { person: people[0] };
      });
    const kept = service(fake);
    expect(await kept.load()).toBeNull();
    expect(kept.people()).toEqual([SAM]);
    expect(kept.invites()).toEqual([INVITE]);
    expect(await kept.setRole('s', 'member')).toBeNull();
    expect(kept.people()[0]?.role).toBe('member');
    expect(fake.paramsOf('people/list')).toEqual([{ limit: 1000 }, { limit: 1000 }]);
  });

  it('makes an invite’s link at the origin, with its email when given', async () => {
    const fake = new FakeSession()
      .answer('people/list', { people: [] })
      .answer('invite/list', { invites: [] })
      .answer('invite/create', { invite: INVITE, code: 'abc' });
    const kept = service(fake);
    const made = await kept.invite('admin', 'alex@example.com', 'https://glave.example');
    expect(made).toEqual({ link: 'https://glave.example/#invite=abc', invite: INVITE });
    await kept.invite('member', null, 'https://glave.example');
    expect(fake.paramsOf('invite/create')).toEqual([
      { role: 'admin', email: 'alex@example.com' },
      { role: 'member' },
    ]);
  });

  it('says the server’s sentence when it refuses, and the app’s own when it can’t be reached', async () => {
    const fake = new FakeSession()
      .answer('people/list', { people: [] })
      .answer('invite/list', { invites: [] })
      .answer(
        'person/remove',
        new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'only an owner can do that' }),
      )
      .answer('invite/revoke', () => {
        throw new TypeError('offline');
      });
    const kept = service(fake);
    expect(await kept.remove('s')).toBe('only an owner can do that');
    expect(await kept.revoke('i')).toBe('glave could not be reached. Try again.');
  });
});
