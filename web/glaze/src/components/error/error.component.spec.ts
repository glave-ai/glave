import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzErrorComponent } from './error.component';

@Component({
  imports: [GlzErrorComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-error>Give it a name.</glz-error>`,
})
class HostComponent {}

describe('GlzErrorComponent', () => {
  it('shows its message', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    expect((fixture.nativeElement as HTMLElement).textContent).toContain('Give it a name.');
  });
});
