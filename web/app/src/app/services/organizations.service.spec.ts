import { TestBed } from '@angular/core/testing';
import type { OrganizationInfo } from '@glave/client';

import { SessionService } from './session.service';
import { OrganizationsService } from './organizations.service';
import { FakeSession, provideFakeSession } from '../testing/fake-session';

const OWN: OrganizationInfo = { id: 'own', people: 1, own: true, role: 'owner', current: true };
const ACME: OrganizationInfo = { id: 'acme', people: 12, role: 'member', owner: 'Dana Lee' };

function service(organizations: readonly OrganizationInfo[]): OrganizationsService {
  const fake = new FakeSession().answer('organization/list', { organizations });
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  return TestBed.inject(OrganizationsService);
}

describe('OrganizationsService', () => {
  it('lists the person’s organizations, and has nothing to switch between with one', async () => {
    const one = service([OWN]);
    await one.load();
    expect(one.organizations()).toEqual([OWN]);
    expect(one.several()).toBe(false);
    expect(one.heading()).toBeNull();
  });

  it('heads the side nav with the team the person works in, never their own', async () => {
    const working = service([
      { ...OWN, current: false },
      { ...ACME, current: true },
    ]);
    await working.load();
    expect(working.several()).toBe(true);
    expect(working.heading()).toBe("Dana Lee's team");
  });

  it('chooses a team by its id, and their own as none, their first', async () => {
    const both = service([OWN, ACME]);
    await both.load();
    const session = TestBed.inject(SessionService);
    const chosen = vi.spyOn(session, 'chooseOrganization').mockReturnValue();
    both.choose(ACME);
    both.choose(OWN);
    expect(chosen.mock.calls).toEqual([['acme'], [null]]);
  });
});
