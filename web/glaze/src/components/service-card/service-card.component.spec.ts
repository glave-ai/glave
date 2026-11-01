import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import type { GlzServiceField } from '../../lib/service-fields';
import { expectAccessible } from '../../testing/axe';
import { GlzServiceCardComponent } from './service-card.component';

const FIELDS: readonly GlzServiceField[] = [
  {
    name: 'label',
    label: 'Label',
    required: true,
    kind: {
      type: 'choice',
      options: [
        { value: 'bug', label: 'Bug' },
        { value: 'idea', label: 'Idea' },
      ],
    },
  },
  { name: 'note', label: 'Note', description: 'Anything to add', kind: { type: 'text' } },
  { name: 'urgent', label: 'Urgent', kind: { type: 'yes_no' } },
  { name: 'count', label: 'Count', kind: { type: 'number', integer: true } },
  {
    name: 'also',
    label: 'Also',
    kind: {
      type: 'choice',
      multiple: true,
      options: [
        { value: 'a', label: 'A' },
        { value: 'b', label: 'B' },
      ],
    },
  },
];

@Component({
  imports: [GlzServiceCardComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-service-card
      testId="service"
      who="GitHub asks you, not the model"
      message="Which label?"
      sendLabel="Send to GitHub"
      note="Your answer goes only to GitHub."
      [fields]="fields()"
      [told]="told()"
      [busy]="busy()"
      (answer)="answers.push($event)"
      (decline)="declined = declined + 1"
    />
  `,
})
class HostComponent {
  readonly fields = signal<readonly GlzServiceField[]>(FIELDS);
  readonly told = signal<string | null>(null);
  readonly busy = signal(false);
  readonly answers: Readonly<Record<string, unknown>>[] = [];
  declined = 0;
}

/** The radio of `value` in `root`. */
function radio(root: HTMLElement, value: string): HTMLInputElement {
  const found = root.querySelector<HTMLInputElement>(`input[type="radio"][value="${value}"]`);
  if (found === null) {
    throw new Error(`no radio ${value}`);
  }
  return found;
}

describe('GlzServiceCardComponent', () => {
  it('says who asks, shows only what it asks for, and sends each value by its name', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const part = (name: string): HTMLElement | null =>
      root.querySelector(`[data-testid="service-${name}"]`);
    expect(part('who')?.textContent).toBe('GitHub asks you, not the model');
    expect(part('message')?.textContent).toBe('Which label?');
    expect(part('note')?.textContent).toBe('Your answer goes only to GitHub.');
    expect(root.querySelectorAll('[data-testid="service-field"]').length).toBe(5);
    // Only the one an answer needs is marked, to the eye and to a screen reader.
    expect(root.querySelectorAll('[data-testid="service-required"]').length).toBe(1);
    expect(root.querySelector('glz-radio-group')?.getAttribute('aria-required')).toBe('true');
    expect(part('missing')?.textContent).toContain('required');
    const send = part('send') as HTMLButtonElement;
    expect(send.textContent.trim()).toBe('Send to GitHub');
    expect(send.disabled).toBe(true);
    radio(root, 'idea').click();
    radio(root, 'yes').click();
    const inputs = root.querySelectorAll<HTMLInputElement>('input.glz-input');
    const [note, count] = inputs;
    if (note === undefined || count === undefined) {
      throw new Error('no inputs');
    }
    note.value = 'soon';
    note.dispatchEvent(new Event('input'));
    count.value = '3';
    count.dispatchEvent(new Event('input'));
    root.querySelector<HTMLInputElement>('input[type="checkbox"]')?.click();
    await fixture.whenStable();
    expect(send.disabled).toBe(false);
    expect(part('missing')).toBeNull();
    send.click();
    expect(fixture.componentInstance.answers).toEqual([
      { label: 'idea', note: 'soon', urgent: true, count: 3, also: ['a'] },
    ]);
    await expectAccessible(root);
  });

  it('declines, and rests as the server line once told', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    root.querySelector<HTMLButtonElement>('[data-testid="service-decline"]')?.click();
    expect(fixture.componentInstance.declined).toBe(1);
    fixture.componentInstance.told.set('You declined GitHub’s question');
    await fixture.whenStable();
    expect(root.querySelector('[data-testid="service-told"]')?.textContent).toBe(
      'You declined GitHub’s question',
    );
    expect(root.querySelector('[data-testid="service-send"]')).toBeNull();
    expect(root.querySelector('[data-testid="service-field"]')).toBeNull();
    await expectAccessible(root);
  });

  it('waits while it sends', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.busy.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const decline = root.querySelector<HTMLButtonElement>('[data-testid="service-decline"]');
    expect(decline?.disabled).toBe(true);
  });
});
