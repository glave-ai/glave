import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import type { GlzOption } from '../../lib/options';
import { expectAccessible } from '../../testing/axe';
import { GlzSegmentedComponent } from './segmented.component';

@Component({
  imports: [GlzSegmentedComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-segmented testId="group" label="Group by" [options]="options" [(value)]="grouping" />
  `,
})
class HostComponent {
  readonly options: readonly GlzOption[] = [
    { value: 'status', label: 'Status' },
    { value: 'project', label: 'Project' },
  ];
  readonly grouping = signal<string | null>('project');
}

function radios(root: HTMLElement): readonly HTMLInputElement[] {
  return Array.from(root.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
}

describe('GlzSegmentedComponent', () => {
  it('names a group of native radios by its label, the value chosen checked', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const host = root.querySelector('[data-testid="group"]');
    const group = root.querySelector('[role="radiogroup"]');
    expect(host?.tagName.toLowerCase()).toBe('glz-segmented');
    expect(
      root.querySelector(`#${group?.getAttribute('aria-labelledby') ?? ''}`)?.textContent,
    ).toBe('Group by');
    const [status, project] = radios(root);
    expect(radios(root).map((radio) => radio.labels?.[0]?.textContent.trim())).toEqual([
      'Status',
      'Project',
    ]);
    expect(status?.name).toBe(project?.name);
    expect([status?.checked, project?.checked]).toEqual([false, true]);
    await expectAccessible(root);
  });

  it('binds the choice both ways', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const [status, project] = radios(root);
    status?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.grouping()).toBe('status');
    expect([status?.checked, project?.checked]).toEqual([true, false]);
    fixture.componentInstance.grouping.set('project');
    await fixture.whenStable();
    expect([status?.checked, project?.checked]).toEqual([false, true]);
  });
});
