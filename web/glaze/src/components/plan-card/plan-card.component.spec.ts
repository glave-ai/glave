import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { type GlzPlanStep, GlzPlanCardComponent } from './plan-card.component';

const PROPOSED: readonly GlzPlanStep[] = [
  { step: 'Read how flags are parsed', status: 'pending' },
  { step: 'Add --verbose', status: 'pending' },
];

const BUILDING: readonly GlzPlanStep[] = [
  { step: 'Read how flags are parsed', status: 'completed' },
  { step: 'Add --verbose', status: 'in_progress' },
  { step: 'Run the tests', status: 'pending' },
];

@Component({
  imports: [GlzPlanCardComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-plan-card
      testId="plan"
      [summary]="summary()"
      [steps]="steps()"
      [proposed]="proposed()"
      [busy]="busy()"
      [size]="size()"
      (build)="did.push('build')"
      (revise)="did.push('revise')"
      (edit)="edits.push($event)"
    />
  `,
})
class HostComponent {
  readonly summary = signal('Proposed plan: 2 steps');
  readonly steps = signal<readonly GlzPlanStep[]>(PROPOSED);
  readonly proposed = signal(true);
  readonly busy = signal(false);
  readonly size = signal<'md' | 'sm'>('md');
  readonly did: string[] = [];
  readonly edits: (readonly string[])[] = [];
}

function all(root: HTMLElement, testId: string): readonly HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(`[data-testid="${testId}"]`));
}

function one(root: HTMLElement, testId: string): HTMLElement | undefined {
  return all(root, testId)[0];
}

function text(element: Element | null | undefined): string {
  return element?.textContent.replace(/\s+/gu, ' ').trim() ?? '';
}

async function rendered(): Promise<{
  readonly fixture: ReturnType<typeof TestBed.createComponent<HostComponent>>;
  readonly root: HTMLElement;
}> {
  const fixture = TestBed.createComponent(HostComponent);
  await fixture.whenStable();
  return { fixture, root: fixture.nativeElement as HTMLElement };
}

/** Types `value` into `field`, as the person would. */
function type(field: HTMLElement | undefined, value: string): void {
  const input = field as HTMLInputElement;
  input.value = value;
  input.dispatchEvent(new Event('input'));
}

describe('GlzPlanCardComponent', () => {
  it('a proposal needs the person: its sentence, its steps, then Build it first', async () => {
    const { fixture, root } = await rendered();
    const card = root.querySelector('glz-plan-card');
    expect(card?.getAttribute('role')).toBe('group');
    const names = (card?.getAttribute('aria-labelledby') ?? '').split(' ');
    expect(names.map((id) => text(document.getElementById(id)))).toEqual([
      'Needs you',
      'Proposed plan: 2 steps',
    ]);
    expect(all(root, 'plan-step').map(text)).toEqual([
      'Read how flags are parsed',
      'Add --verbose',
    ]);
    const buttons = Array.from(root.querySelectorAll('.actions button'));
    expect(buttons.map(text)).toEqual(['Build it', 'Change it', 'Edit']);
    expect(buttons[0]?.classList).toContain('glz-button--primary');
    await expectAccessible(root);
    one(root, 'plan-build')?.click();
    one(root, 'plan-change')?.click();
    expect(fixture.componentInstance.did).toEqual(['build', 'revise']);
  });

  it('edits its steps in place: changed, removed and added, then saved trimmed, none blank', async () => {
    const { fixture, root } = await rendered();
    one(root, 'plan-edit')?.click();
    await fixture.whenStable();
    expect(root.querySelector('.label')).toBeNull();
    const fields = all(root, 'plan-input') as readonly HTMLInputElement[];
    expect(fields.map((field) => field.value)).toEqual([
      'Read how flags are parsed',
      'Add --verbose',
    ]);
    expect(fields.map((field) => field.getAttribute('aria-label'))).toEqual(['Step 1', 'Step 2']);
    const remove = one(root, 'plan-remove');
    expect(remove?.getAttribute('aria-label')).toBe('Remove step');
    const describing = document.getElementById(remove?.getAttribute('aria-describedby') ?? '');
    expect(describing?.textContent).toBe('Read how flags are parsed');
    await expectAccessible(root);
    type(fields[1], ' Add --verbose and -v ');
    remove?.click();
    await fixture.whenStable();
    one(root, 'plan-add')?.click();
    one(root, 'plan-add')?.click();
    await fixture.whenStable();
    const now = all(root, 'plan-input');
    expect(now).toHaveLength(3);
    type(now[1], 'Run the tests');
    one(root, 'plan-save')?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.edits).toEqual([['Add --verbose and -v', 'Run the tests']]);
    expect(all(root, 'plan-input')).toHaveLength(0);
  });

  it('the keyboard follows the edit: into its fields, to what is added or left, then back', async () => {
    const { fixture, root } = await rendered();
    one(root, 'plan-edit')?.click();
    await fixture.whenStable();
    expect(document.activeElement).toBe(all(root, 'plan-input')[0]);
    one(root, 'plan-add')?.click();
    await fixture.whenStable();
    expect(document.activeElement).toBe(all(root, 'plan-input')[2]);
    all(root, 'plan-remove')[2]?.click();
    await fixture.whenStable();
    expect(document.activeElement).toBe(all(root, 'plan-input')[1]);
    for (const left of [1, 0]) {
      all(root, 'plan-remove')[0]?.click();
      await fixture.whenStable();
      expect(all(root, 'plan-input')).toHaveLength(left);
    }
    expect(document.activeElement).toBe(one(root, 'plan-add'));
    one(root, 'plan-cancel')?.click();
    await fixture.whenStable();
    expect(document.activeElement?.classList).toContain('summary');
  });

  it('cancelling an edit keeps the plan as it was, and nothing is saved', async () => {
    const { fixture, root } = await rendered();
    one(root, 'plan-edit')?.click();
    await fixture.whenStable();
    type(one(root, 'plan-input'), 'Something else');
    one(root, 'plan-cancel')?.click();
    await fixture.whenStable();
    expect(all(root, 'plan-step').map(text)).toEqual([
      'Read how flags are parsed',
      'Add --verbose',
    ]);
    expect(fixture.componentInstance.edits).toEqual([]);
  });

  it('a plan being built needs no one: each step ticks off with its mark', async () => {
    const { fixture, root } = await rendered();
    fixture.componentInstance.proposed.set(false);
    fixture.componentInstance.steps.set(BUILDING);
    fixture.componentInstance.summary.set('Plan: 1 of 3 steps done');
    await fixture.whenStable();
    expect(root.querySelector('.label')).toBeNull();
    expect(root.querySelector('.actions')).toBeNull();
    const card = root.querySelector('glz-plan-card');
    expect(text(document.getElementById(card?.getAttribute('aria-labelledby') ?? ''))).toBe(
      'Plan: 1 of 3 steps done',
    );
    const marks = all(root, 'plan-step').map((step) =>
      step.querySelector('glz-icon')?.getAttribute('aria-label'),
    );
    expect(marks).toEqual(['Done', 'Doing', 'To do']);
    expect(all(root, 'plan-step')[1]?.querySelector('glz-icon')?.classList).toContain('glz-spin');
    await expectAccessible(root);
  });

  it('is tighter in a column of facts, small', async () => {
    const { fixture, root } = await rendered();
    const card = root.querySelector('glz-plan-card');
    expect(card?.classList).not.toContain('glz-plan-card--sm');
    fixture.componentInstance.size.set('sm');
    await fixture.whenStable();
    expect(card?.classList).toContain('glz-plan-card--sm');
    await expectAccessible(root);
  });

  it('waits while busy', async () => {
    const { fixture, root } = await rendered();
    fixture.componentInstance.busy.set(true);
    await fixture.whenStable();
    expect(root.querySelector('glz-plan-card')?.getAttribute('aria-busy')).toBe('true');
    for (const testId of ['plan-build', 'plan-change', 'plan-edit']) {
      expect((one(root, testId) as HTMLButtonElement | undefined)?.disabled).toBe(true);
    }
  });
});
