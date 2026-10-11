import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { type GlzComposed, GlzComposerComponent } from './composer.component';

@Component({
  imports: [GlzComposerComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-composer
      [(value)]="draft"
      [(files)]="files"
      [busy]="busy()"
      (send)="sent.push($event)"
      (stop)="stops.set(stops() + 1)"
    >
      <span glzComposerStart data-testid="model">Hope</span>
      <span glzComposerEnd data-testid="cloud">Cloud</span>
    </glz-composer>
  `,
})
class HostComponent {
  readonly draft = signal('');
  readonly files = signal<readonly File[]>([]);
  readonly busy = signal(false);
  readonly stops = signal(0);
  readonly sent: GlzComposed[] = [];
}

function find<T extends HTMLElement = HTMLElement>(root: HTMLElement, testId: string): T {
  const found = root.querySelector<T>(`[data-testid="${testId}"]`);
  if (found === null) {
    throw new Error(`no ${testId}`);
  }
  return found;
}

function type(textarea: HTMLTextAreaElement, text: string): void {
  textarea.value = text;
  textarea.dispatchEvent(new Event('input'));
}

function enter(textarea: HTMLTextAreaElement, init: KeyboardEventInit = {}): KeyboardEvent {
  const event = new KeyboardEvent('keydown', { key: 'Enter', cancelable: true, ...init });
  textarea.dispatchEvent(event);
  return event;
}

describe('GlzComposerComponent', () => {
  it('sends the trimmed draft on Enter and clears it; Shift+Enter is a new line', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const input = find<HTMLTextAreaElement>(root, 'composer-input');
    expect(find<HTMLButtonElement>(root, 'composer-send').disabled).toBe(true);
    expect(input.getAttribute('aria-label')).toBe('Message');
    expect(find(root, 'model').closest('.start')).not.toBeNull();
    expect(find(root, 'cloud').closest('.end')).not.toBeNull();
    await expectAccessible(root);

    type(input, '  Plan the launch ');
    await fixture.whenStable();
    expect(fixture.componentInstance.draft()).toBe('  Plan the launch ');
    expect(enter(input, { shiftKey: true }).defaultPrevented).toBe(false);
    expect(enter(input).defaultPrevented).toBe(true);
    await fixture.whenStable();
    expect(fixture.componentInstance.sent).toEqual([{ text: 'Plan the launch', files: [] }]);
    expect(fixture.componentInstance.draft()).toBe('');
    expect(input.value).toBe('');
  });

  it('does not send while composing text with an input method', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const input = find<HTMLTextAreaElement>(fixture.nativeElement, 'composer-input');
    type(input, 'にほん');
    enter(input, { isComposing: true });
    expect(fixture.componentInstance.sent).toEqual([]);
  });

  it('shows Stop while busy, stops on click or Escape, and keeps the draft', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.busy.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="composer-send"]')).toBeNull();
    find<HTMLButtonElement>(root, 'composer-stop').click();
    const input = find<HTMLTextAreaElement>(root, 'composer-input');
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(fixture.componentInstance.stops()).toBe(2);
    type(input, 'next');
    enter(input);
    expect(fixture.componentInstance.sent).toEqual([]);
  });

  it('attaches files from the picker, lists them, removes and sends them', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const picker = find<HTMLInputElement>(root, 'composer-file-input');
    const click = vi.spyOn(picker, 'click');
    find<HTMLButtonElement>(root, 'composer-attach').click();
    expect(click).toHaveBeenCalledOnce();
    const notes = new File(['hello'], 'notes.txt');
    const plan = new File(['plan'], 'plan.md');
    Object.defineProperty(picker, 'files', { configurable: true, value: [notes, plan] });
    picker.dispatchEvent(new Event('change'));
    await fixture.whenStable();
    expect(find(root, 'composer-files').textContent).toContain('notes.txt');
    const remove = root.querySelectorAll<HTMLButtonElement>('[data-testid="composer-remove-file"]');
    expect(remove[0]?.getAttribute('aria-label')).toBe('Remove notes.txt');
    remove[0]?.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.files()).toEqual([plan]);
    find<HTMLButtonElement>(root, 'composer-send').click();
    expect(fixture.componentInstance.sent).toEqual([{ text: '', files: [plan] }]);
    expect(fixture.componentInstance.files()).toEqual([]);
  });
});
