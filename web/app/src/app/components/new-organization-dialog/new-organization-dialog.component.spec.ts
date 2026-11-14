import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type OrganizationInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { SessionService } from '../../services/session.service';
import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { settle } from '../../testing/links';
import { NewOrganizationDialogComponent } from './new-organization-dialog.component';

const NURFZ: OrganizationInfo = { id: 'nurfz', name: 'Nurfz', people: 1, role: 'owner' };

function byTestId<T extends HTMLElement>(testId: string): T | null {
  return document.querySelector<T>(`[data-testid="${testId}"]`);
}

function type(value: string): void {
  const name = byTestId<HTMLInputElement>('new-organization-name');
  if (name === null) {
    throw new Error('no name field');
  }
  name.value = value;
  name.dispatchEvent(new Event('input'));
}

async function opened(
  fake: FakeSession,
): Promise<ComponentFixture<NewOrganizationDialogComponent>> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(NewOrganizationDialogComponent);
  fixture.componentRef.setInput('open', true);
  await settle(fixture);
  return fixture;
}

describe('NewOrganizationDialogComponent', () => {
  it('creates an organization by its name, trimmed, then works in it', async () => {
    const fake = new FakeSession().answer('organization/create', { organization: NURFZ });
    const chosen = vi.fn<(id: string) => void>();
    const fixture = await opened(fake);
    vi.spyOn(TestBed.inject(SessionService), 'chooseOrganization').mockImplementation(chosen);
    await expectAccessible(document.body);
    expect(byTestId<HTMLInputElement>('new-organization-name')?.maxLength).toBe(80);
    type('  Nurfz  ');
    await settle(fixture);
    byTestId('new-organization-create')?.click();
    await settle(fixture);
    expect(fake.paramsOf('organization/create')).toEqual([{ name: 'Nurfz' }]);
    expect(chosen).toHaveBeenCalledWith('nurfz');
    expect(fixture.componentInstance.open()).toBe(false);
  });

  it('creates nothing without a name', async () => {
    const fake = new FakeSession();
    const fixture = await opened(fake);
    type('   ');
    await settle(fixture);
    expect(byTestId<HTMLButtonElement>('new-organization-create')?.disabled).toBe(true);
    document.querySelector('form')?.dispatchEvent(new Event('submit', { cancelable: true }));
    await settle(fixture);
    expect(fake.paramsOf('organization/create')).toEqual([]);
  });

  it('says why when the server refuses, and stays open', async () => {
    const refused = new ApiError({
      code: ErrorCode.LIMIT,
      message: "you've created 10 organizations, the most anyone can",
    });
    const fake = new FakeSession().answer('organization/create', refused);
    const fixture = await opened(fake);
    type('Eleventh');
    await settle(fixture);
    byTestId('new-organization-create')?.click();
    await settle(fixture);
    expect(byTestId('new-organization-problem')?.textContent).toContain('the most anyone can');
    expect(fixture.componentInstance.open()).toBe(true);
    await expectAccessible(document.body);
  });
});
