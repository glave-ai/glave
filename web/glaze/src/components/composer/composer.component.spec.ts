import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzComposerComponent, type GlzComposerSize } from './composer.component';

@Component({
  imports: [GlzComposerComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-composer
      testId="composer"
      [(value)]="draft"
      [busy]="busy()"
      [size]="size()"
      (send)="sent.push($event)"
      (stop)="stops.set(stops() + 1)"
    >
      <span glzComposerStart data-testid="where">A chat</span>
      <span glzComposerStatus data-testid="model">Hope</span>
    </glz-composer>
  `,
})
class HostComponent {
  readonly draft = signal('');
  readonly busy = signal(false);
  readonly size = signal<GlzComposerSize>('md');
  readonly stops = signal(0);
  readonly sent: string[] = [];
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
    // In the box: where it goes and send; under it, the status line.
    expect(find(root, 'where').closest('.box .start')).not.toBeNull();
    expect(find(root, 'composer-send').closest('.box')).not.toBeNull();
    expect(find(root, 'model').closest('.box')).toBeNull();
    expect(find(root, 'model').closest('[data-testid="composer-status"]')).not.toBeNull();
    await expectAccessible(root);

    type(input, '  Plan the launch ');
    await fixture.whenStable();
    expect(fixture.componentInstance.draft()).toBe('  Plan the launch ');
    expect(enter(input, { shiftKey: true }).defaultPrevented).toBe(false);
    expect(enter(input).defaultPrevented).toBe(true);
    await fixture.whenStable();
    expect(fixture.componentInstance.sent).toEqual(['Plan the launch']);
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
    expect(fixture.componentInstance.sent).toEqual([]);
    await expectAccessible(root);
  });

  it('sends while busy: on Enter, or with Send beside Stop once there is something to send', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.busy.set(true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const input = find<HTMLTextAreaElement>(root, 'composer-input');
    type(input, ' use 4000 instead ');
    await fixture.whenStable();
    const send = find<HTMLButtonElement>(root, 'composer-send');
    const stop = find<HTMLButtonElement>(root, 'composer-stop');
    // Stop stays at the end, where Send was.
    expect(send.nextElementSibling).toBe(stop);
    await expectAccessible(root);
    send.click();
    await fixture.whenStable();
    expect(fixture.componentInstance.sent).toEqual(['use 4000 instead']);
    expect(root.querySelector('[data-testid="composer-send"]')).toBeNull();
    type(input, 'and keep the comment');
    expect(enter(input).defaultPrevented).toBe(true);
    expect(fixture.componentInstance.sent).toEqual(['use 4000 instead', 'and keep the comment']);
    expect(fixture.componentInstance.stops()).toBe(0);
  });

  it('opens its status line from the options button, as a phone shows it', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const options = find<HTMLButtonElement>(root, 'composer-options');
    const status = find(root, 'composer-status');
    expect(options.getAttribute('aria-label')).toBe('Options');
    expect(options.getAttribute('popovertarget')).toBe(status.id);
    expect(status.hasAttribute('popover')).toBe(true);
    expect(status.getAttribute('role')).toBe('group');
    expect(status.getAttribute('aria-label')).toBe('Options');
  });

  it('is a reply’s size unless it starts a thread', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(find(root, 'composer').classList).not.toContain('glz-composer--lg');
    fixture.componentInstance.size.set('lg');
    await fixture.whenStable();
    expect(find(root, 'composer').classList).toContain('glz-composer--lg');
    await expectAccessible(root);
  });
});
