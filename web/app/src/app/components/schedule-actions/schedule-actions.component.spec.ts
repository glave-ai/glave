import { LiveAnnouncer } from '@angular/cdk/a11y';
import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { type ComponentFixture, TestBed } from '@angular/core/testing';
import { ApiError, ErrorCode, type ScheduleInfo } from '@glave/client';
import { expectAccessible } from '@glave/glaze/testing';

import { FakeSession, provideFakeSession } from '../../testing/fake-session';
import { schedule } from '../../testing/fixtures';
import { ScheduleActionsComponent } from './schedule-actions.component';

const NEXT = '2026-11-17T02:00:00.000Z';

@Component({
  imports: [ScheduleActionsComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <app-schedule-actions
      testId="actions"
      [schedule]="schedule()"
      [menu]="menu()"
      [(error)]="error"
      (changed)="change($event)"
      (removed)="remove()"
    />
  `,
})
class HostComponent {
  readonly schedule = signal<ScheduleInfo>(schedule('s1', NEXT));
  readonly menu = signal(false);
  readonly error = signal<string | null>(null);
  readonly changed: ScheduleInfo[] = [];
  removed = 0;

  change(now: ScheduleInfo): void {
    this.changed.push(now);
    this.schedule.set(now);
  }

  remove(): void {
    this.removed += 1;
  }
}

/** The fake's answer to a pause: the schedule paused or resumed as asked. */
function paused(params: unknown): { readonly schedule: ScheduleInfo } {
  const { paused: now } = params as { readonly paused: boolean };
  return { schedule: { ...schedule('s1', now ? null : NEXT), paused: now } };
}

async function render(
  fake: FakeSession,
  menu = false,
): Promise<{
  readonly root: HTMLElement;
  readonly fixture: ComponentFixture<HostComponent>;
  readonly announced: string[];
}> {
  const announced: string[] = [];
  const announce = (message: string): Promise<void> => {
    announced.push(message);
    return Promise.resolve();
  };
  TestBed.configureTestingModule({
    providers: [...provideFakeSession(fake), { provide: LiveAnnouncer, useValue: { announce } }],
  });
  const fixture = TestBed.createComponent(HostComponent);
  fixture.componentInstance.menu.set(menu);
  await fixture.whenStable();
  return { root: fixture.nativeElement as HTMLElement, fixture, announced };
}

function find(root: ParentNode, testId: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(`[data-testid="${testId}"]`);
}

/** Whether the dialog asking before Remove is open. */
function confirming(root: HTMLElement): boolean {
  return (
    root.querySelector<HTMLDialogElement>('dialog[data-testid="actions-confirm"]')?.open ?? false
  );
}

async function settle(fixture: ComponentFixture<HostComponent>): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve));
  await fixture.whenStable();
}

describe('ScheduleActionsComponent', () => {
  it('pauses and resumes, waiting while the server answers, and says what came of it', async () => {
    const fake = new FakeSession();
    const answer = fake.held('schedule/pause');
    const { root, fixture, announced } = await render(fake);
    const toggle = find(root, 'actions-pause');
    expect(toggle?.textContent.trim()).toBe('Pause');
    expect(toggle?.getAttribute('aria-label')).toBe('Pause Audit the dependencies');
    expect(find(root, 'actions-remove')?.getAttribute('aria-label')).toBe(
      'Remove Audit the dependencies',
    );
    await expectAccessible(root);
    toggle?.click();
    await fixture.whenStable();
    // One change at a time: a second press while the first is on its way asks nothing more.
    expect(root.querySelector('app-schedule-actions')?.getAttribute('aria-busy')).toBe('true');
    expect(toggle?.getAttribute('aria-disabled')).toBe('true');
    toggle?.click();
    expect(fake.paramsOf('schedule/pause')).toEqual([{ schedule: 's1', paused: true }]);
    answer(paused({ paused: true }));
    await settle(fixture);
    expect(fixture.componentInstance.changed.map((each) => each.paused)).toEqual([true]);
    expect(announced).toEqual(['Paused']);
    expect(toggle?.textContent.trim()).toBe('Resume');
    expect(toggle?.getAttribute('aria-label')).toBe('Resume Audit the dependencies');
    expect(toggle?.hasAttribute('aria-disabled')).toBe(false);
  });

  it('puts what the server refused a change with in the error, and clears it at the next', async () => {
    const fake = new FakeSession().answer(
      'schedule/pause',
      new ApiError({ code: ErrorCode.NO_SCHEDULE, message: 'no such schedule' }),
    );
    const { root, fixture, announced } = await render(fake);
    find(root, 'actions-pause')?.click();
    await settle(fixture);
    expect(fixture.componentInstance.error()).toBe('no such schedule');
    expect(fixture.componentInstance.changed).toEqual([]);
    expect(announced).toEqual([]);
    fake.answer('schedule/pause', paused);
    find(root, 'actions-pause')?.click();
    await settle(fixture);
    expect(fixture.componentInstance.error()).toBeNull();
  });

  it('removes the schedule once the person says yes, and not before', async () => {
    const fake = new FakeSession();
    const { root, fixture, announced } = await render(fake);
    find(root, 'actions-remove')?.click();
    await fixture.whenStable();
    find(document.body, 'actions-confirm-cancel')?.click();
    await settle(fixture);
    expect(fake.paramsOf('schedule/delete')).toEqual([]);
    expect(confirming(root)).toBe(false);
    find(root, 'actions-remove')?.click();
    await fixture.whenStable();
    expect(confirming(root)).toBe(true);
    expect(find(document.body, 'actions-confirm')?.textContent).toContain(
      'Its chat stays, with its past runs.',
    );
    find(document.body, 'actions-confirm-ok')?.click();
    await settle(fixture);
    expect(fake.paramsOf('schedule/delete')).toEqual([{ schedule: 's1' }]);
    expect(fixture.componentInstance.removed).toBe(1);
    expect(announced).toEqual(['Removed']);
  });

  it('offers both under a ⋯ menu, named for the schedule', async () => {
    const fake = new FakeSession().answer('schedule/pause', paused);
    const { root, fixture } = await render(fake, true);
    expect(find(root, 'actions-pause')).toBeNull();
    const trigger = find(root, 'actions-menu-trigger');
    expect(trigger?.getAttribute('aria-label')).toBe('Options for Audit the dependencies');
    await expectAccessible(root);
    trigger?.click();
    await fixture.whenStable();
    const items = [
      ...document.querySelectorAll<HTMLElement>('[data-testid="actions-menu-action"]'),
    ];
    expect(items.map((item) => item.textContent.trim())).toEqual(['Pause', 'Remove']);
    items[0]?.click();
    await settle(fixture);
    expect(fake.paramsOf('schedule/pause')).toEqual([{ schedule: 's1', paused: true }]);
    find(root, 'actions-menu-trigger')?.click();
    await fixture.whenStable();
    const again = [
      ...document.querySelectorAll<HTMLElement>('[data-testid="actions-menu-action"]'),
    ];
    expect(again.map((item) => item.textContent.trim())).toEqual(['Resume', 'Remove']);
    again[1]?.click();
    await fixture.whenStable();
    expect(confirming(root)).toBe(true);
  });
});
