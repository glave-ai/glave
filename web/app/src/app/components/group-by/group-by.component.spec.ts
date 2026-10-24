import { TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { GroupByComponent } from './group-by.component';

async function render(fake: FakeSession): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<GroupByComponent>>;
}> {
  TestBed.configureTestingModule({ providers: provideFakeSession(fake) });
  const fixture = TestBed.createComponent(GroupByComponent);
  await fixture.whenStable();
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function radio(root: HTMLElement, label: string): HTMLInputElement | undefined {
  return Array.from(root.querySelectorAll<HTMLInputElement>('input[type="radio"]')).find(
    (each) => each.labels?.[0]?.textContent.trim() === label,
  );
}

describe('GroupByComponent', () => {
  it('switches between Status and Project, as the person chose, and keeps a new choice', async () => {
    const fake = new FakeSession().answer('grouping/get', { grouping: 'status' });
    const { root, fixture } = await render(fake);
    expect(root.querySelector('[data-testid="nav-group"]')?.textContent).toContain('Group by');
    expect(radio(root, 'Status')?.checked).toBe(true);
    expect(radio(root, 'Project')?.checked).toBe(false);
    await expectAccessible(root);
    radio(root, 'Project')?.click();
    await fixture.whenStable();
    expect(fake.paramsOf('grouping/set')).toEqual([{ grouping: 'project' }]);
    expect(radio(root, 'Project')?.checked).toBe(true);
    expect(root.querySelector('[data-testid="nav-group-error"]')).toBeNull();
  });

  it('puts the choice back and says why when it is not kept', async () => {
    const fake = new FakeSession().answer(
      'grouping/set',
      new ApiError({ code: ErrorCode.INTERNAL, message: 'the store is read-only' }),
    );
    const { root, fixture } = await render(fake);
    radio(root, 'Status')?.click();
    await fixture.whenStable();
    await new Promise((resolve) => setTimeout(resolve));
    await fixture.whenStable();
    expect(radio(root, 'Project')?.checked).toBe(true);
    const error = root.querySelector('[data-testid="nav-group-error"]');
    expect(error?.getAttribute('role')).toBe('alert');
    expect(error?.textContent.trim()).toBe('The store is read-only.');
    await expectAccessible(root);
  });
});
