import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type InviteInfo, type PersonInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { PeopleComponent } from './people.component';

const DANA: PersonInfo = {
  id: 'p-dana',
  name: 'Dana Lee',
  email: 'dana@example.com',
  role: 'owner',
  you: true,
};
const SAM: PersonInfo = {
  id: 'p-sam',
  name: 'Sam Ortiz',
  email: 'sam@example.com',
  role: 'admin',
  roles: ['owner', 'admin', 'member'],
  removable: true,
};
const PRIYA: PersonInfo = {
  id: 'p-priya',
  email: 'priya@example.com',
  role: 'member',
  roles: ['owner', 'admin', 'member'],
  removable: true,
};

const WEEK = 7 * 86_400_000;

function invite(id: string, by: string, email?: string): InviteInfo {
  const now = Date.now();
  return {
    id,
    role: 'member',
    by: { kind: 'person', id: by },
    made: new Date(now).toISOString(),
    expires: new Date(now + WEEK - 3_600_000).toISOString(),
    ...(email === undefined ? {} : { email }),
  };
}

async function settle(fixture: { whenStable(): Promise<unknown> }): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    await new Promise((resolve) => setTimeout(resolve));
  }
  await fixture.whenStable();
}

function click(root: ParentNode, testId: string, at = 0): void {
  root.querySelectorAll<HTMLElement>(`[data-testid="${testId}"]`)[at]?.click();
}

async function render(
  fake: FakeSession,
): Promise<{ readonly fixture: ComponentFixture<PeopleComponent>; readonly root: HTMLElement }> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(PeopleComponent);
  await settle(fixture);
  return { fixture, root: fixture.nativeElement as HTMLElement };
}

describe('PeopleComponent', () => {
  it('lists the people with their roles, and the invites waiting with who made them', async () => {
    const fake = new FakeSession()
      .answer('people/list', { people: [DANA, SAM, PRIYA] })
      .answer('invite/list', {
        invites: [invite('i1', 'p-dana', 'alex@example.com'), invite('i2', 'p-sam')],
      });
    const { root } = await render(fake);
    const people = root.querySelectorAll('[data-testid="people-person"]');
    expect(people.length).toBe(3);
    expect(people[0]?.textContent).toContain('Owner · you');
    expect(people[0]?.querySelector('[data-testid="people-remove"]')).toBeNull();
    expect(people[1]?.querySelector('[data-testid="people-role"]')).not.toBeNull();
    // Someone with no name is named by their email.
    expect(people[2]?.querySelector('[data-testid="people-name"]')?.textContent).toContain(
      'priya@example.com',
    );
    expect(root.querySelector('[data-testid="people-count"]')?.textContent).toContain(
      '3 people · 2 invites waiting',
    );
    const waiting = root.querySelectorAll('[data-testid="people-waiting"]');
    expect(waiting[0]?.textContent).toContain('alex@example.com');
    expect(waiting[0]?.textContent).toContain('invited by you');
    expect(waiting[1]?.textContent).toContain('A link, no email');
    expect(waiting[1]?.textContent).toContain('invited by Sam Ortiz');
    await expectAccessible(root);
  });

  it('removes someone once the reader says so, and cancels an invite', async () => {
    let people: readonly PersonInfo[] = [DANA, SAM];
    let invites: readonly InviteInfo[] = [invite('i1', 'p-dana')];
    const fake = new FakeSession()
      .answer('people/list', () => ({ people }))
      .answer('invite/list', () => ({ invites }))
      .answer('person/remove', () => {
        people = [DANA];
        return {};
      })
      .answer('invite/revoke', () => {
        invites = [];
        return {};
      });
    const { fixture, root } = await render(fake);
    click(root, 'people-remove');
    await settle(fixture);
    expect(document.body.textContent).toContain('Remove Sam Ortiz?');
    click(document, 'people-remove-confirm-ok');
    await settle(fixture);
    expect(fake.paramsOf('person/remove')).toEqual([{ person: 'p-sam' }]);
    expect(root.querySelectorAll('[data-testid="people-person"]').length).toBe(1);
    click(root, 'people-cancel-invite');
    await settle(fixture);
    expect(fake.paramsOf('invite/revoke')).toEqual([{ invite: 'i1' }]);
    expect(root.querySelector('[data-testid="people-invites"]')).toBeNull();
  });

  it('says the server’s sentence when it refuses', async () => {
    const fake = new FakeSession()
      .answer('people/list', { people: [DANA, SAM] })
      .answer('invite/list', { invites: [] })
      .answer(
        'person/remove',
        new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'only an owner can do that' }),
      );
    const { fixture, root } = await render(fake);
    click(root, 'people-remove');
    await settle(fixture);
    click(document, 'people-remove-confirm-ok');
    await settle(fixture);
    expect(root.querySelector('[data-testid="people-problem"]')?.textContent).toContain(
      'only an owner can do that',
    );
  });

  it('gives someone a role from their menu, and shows the server’s answer when it refuses', async () => {
    let people: readonly PersonInfo[] = [DANA, SAM, PRIYA];
    const fake = new FakeSession()
      .answer('people/list', () => ({ people }))
      .answer('invite/list', { invites: [] })
      .answer('role/set', () => {
        people = [DANA, SAM, { ...PRIYA, role: 'admin' }];
        return { person: people[2] };
      });
    const { fixture, root } = await render(fake);
    const menus = root.querySelectorAll<HTMLElement>('[data-testid="people-role"] button');
    menus[1]?.click();
    await settle(fixture);
    document.querySelectorAll<HTMLElement>('[role="menuitemradio"]').forEach((item) => {
      if (item.textContent.trim().startsWith('Admin')) {
        item.click();
      }
    });
    await settle(fixture);
    expect(fake.paramsOf('role/set')).toEqual([{ person: 'p-priya', role: 'admin' }]);

    fake.answer(
      'role/set',
      new ApiError({ code: ErrorCode.NOT_ALLOWED, message: 'only an owner can do that' }),
    );
    root.querySelectorAll<HTMLElement>('[data-testid="people-role"] button')[1]?.click();
    await settle(fixture);
    document.querySelectorAll<HTMLElement>('[role="menuitemradio"]').forEach((item) => {
      if (item.textContent.trim().startsWith('Member')) {
        item.click();
      }
    });
    await settle(fixture);
    expect(root.querySelector('[data-testid="people-problem"]')?.textContent).toContain(
      'only an owner can do that',
    );
    // What the server keeps shows again: Priya is still an Admin.
    const shown = root.querySelectorAll('[data-testid="people-role"]')[1];
    expect(shown?.textContent).toContain('Admin');
  });
});
