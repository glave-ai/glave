import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzMessageComponent } from './message.component';

@Component({
  imports: [GlzMessageComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-message from="model" author="Hope" hope [streaming]="streaming()" data-testid="reply">
      <p>Hello</p>
    </glz-message>
    <glz-message from="note" author="glave" data-testid="note">Compacted</glz-message>
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
    expect(reply?.getAttribute('aria-label')).toBe('Hope');
    expect(reply?.getAttribute('aria-busy')).toBe('true');
    expect(reply?.querySelector('.caret')).not.toBeNull();
    expect(reply?.querySelector('glz-avatar .presence')).not.toBeNull();
    expect(root.querySelector('[data-testid="note"] glz-avatar')).toBeNull();
    await expectAccessible(root);

    fixture.componentInstance.streaming.set(false);
    await fixture.whenStable();
    expect(reply?.hasAttribute('aria-busy')).toBe(false);
    expect(reply?.querySelector('.caret')).toBeNull();
  });
});
