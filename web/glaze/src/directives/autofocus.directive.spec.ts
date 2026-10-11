import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzAutofocusDirective } from './autofocus.directive';

@Component({
  imports: [GlzAutofocusDirective],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <input aria-label="Skipped" data-testid="skipped" [glzAutofocus]="false" />
    <textarea aria-label="Message" data-testid="message" glzAutofocus></textarea>
  `,
})
class HostComponent {}

describe('GlzAutofocusDirective', () => {
  it('focuses its element after rendering, when enabled', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    document.body.appendChild(fixture.nativeElement as HTMLElement);
    await fixture.whenStable();
    expect(document.activeElement?.getAttribute('data-testid')).toBe('message');
  });
});
