import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzMessageComponent } from './message.component';

@Component({
  imports: [GlzMessageComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-message from="model" author="Reply" [streaming]="streaming()" data-testid="reply">
      <p>Hello</p>
      <button type="button" glzMessageActions data-testid="copy">Copy</button>
    </glz-message>
    <glz-message from="note" author="Atlas" data-testid="note">Compacted</glz-message>
    <glz-message from="person" author="You" data-testid="mine"><p>Hi</p></glz-message>
  `,
})
class HostComponent {
  readonly streaming = signal(true);
}

describe('GlzMessageComponent', () => {
  it('is an article named by its author, busy while streaming', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const reply = root.querySelector('[data-testid="reply"]');
    expect(reply?.getAttribute('role')).toBe('article');
    expect(reply?.getAttribute('aria-label')).toBe('Reply');
    expect(reply?.getAttribute('aria-busy')).toBe('true');
    expect(reply?.querySelector('.caret')).not.toBeNull();
    expect(root.querySelector('glz-avatar')).toBeNull();
    const mine = root.querySelector('[data-testid="mine"]');
    expect(mine?.getAttribute('aria-label')).toBe('You');
    expect(mine?.classList).toContain('from-person');
    await expectAccessible(root);

    fixture.componentInstance.streaming.set(false);
    await fixture.whenStable();
    expect(reply?.hasAttribute('aria-busy')).toBe(false);
    expect(reply?.querySelector('.caret')).toBeNull();
  });

  it('names only a note over it: a reply is plain text, the person’s a bubble', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="reply"] .author')).toBeNull();
    expect(root.querySelector('[data-testid="mine"] .author')).toBeNull();
    expect(root.querySelector('[data-testid="note"] .author')?.textContent).toBe('Atlas');
  });

  it('hides its actions until keyboard focus is in the message', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const copy = root.querySelector<HTMLElement>('[data-testid="copy"]');
    const actions = copy?.parentElement ?? root;
    expect(getComputedStyle(actions).opacity).toBe('0');
    // jsdom counts any focus as the keyboard's; that a click's focus shows nothing is checked
    // in browsers, end to end.
    copy?.focus();
    expect(getComputedStyle(actions).opacity).toBe('1');
    copy?.blur();
    expect(getComputedStyle(actions).opacity).toBe('0');
  });

  it('puts its actions under the body, outside the bubble', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const copy = root.querySelector('[data-testid="copy"]');
    expect(copy?.parentElement?.classList).toContain('actions');
    expect(root.querySelector('[data-testid="reply"] .body [data-testid="copy"]')).toBeNull();
  });
});
