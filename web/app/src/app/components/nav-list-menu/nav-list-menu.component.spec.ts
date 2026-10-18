import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { NavListMenuComponent } from './nav-list-menu.component';

async function render(): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ReturnType<typeof TestBed.createComponent<NavListMenuComponent>>;
}> {
  const fixture = TestBed.createComponent(NavListMenuComponent);
  fixture.componentRef.setInput('testId', 'more');
  fixture.componentRef.setInput('label', 'Options for glave');
  fixture.componentRef.setInput('actions', [{ value: 'rename', label: 'Rename' }]);
  await fixture.whenStable();
  // A menu that opens before its trigger's first tick closes again (the CDK's focus check).
  await new Promise((resolve) => setTimeout(resolve));
  return { root: fixture.nativeElement as HTMLElement, fixture };
}

function items(testId: string): readonly HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>(`[data-testid="${testId}"]`));
}

describe('NavListMenuComponent', () => {
  afterEach(() => {
    document.body.click();
  });

  it('offers the list’s actions, then sorting it by recent activity or by name', async () => {
    const { root, fixture } = await render();
    const trigger = root.querySelector<HTMLButtonElement>('[data-testid="more-trigger"]');
    expect(trigger?.getAttribute('aria-label')).toBe('Options for glave');
    await expectAccessible(root);
    trigger?.click();
    await fixture.whenStable();
    expect(items('more-action').map((item) => item.textContent.trim())).toEqual(['Rename']);
    expect(document.querySelector('[role="group"]')?.textContent).toContain('Sort by');
    expect(
      items('more-choice').map((item) => [
        item.textContent.trim(),
        item.getAttribute('aria-checked'),
      ]),
    ).toEqual([
      ['Recent activity', 'true'],
      ['Name', 'false'],
    ]);
    items('more-choice')[1]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.sort()).toBe('name');
  });

  it('emits the action picked', async () => {
    const { root, fixture } = await render();
    const picked: string[] = [];
    fixture.componentInstance.action.subscribe((value) => picked.push(value));
    root.querySelector<HTMLButtonElement>('[data-testid="more-trigger"]')?.click();
    await fixture.whenStable();
    items('more-action')[0]?.click();
    await fixture.whenStable();
    expect(picked).toEqual(['rename']);
    expect(fixture.componentInstance.sort()).toBe('recent');
  });
});
