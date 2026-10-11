import { ChangeDetectionStrategy, Component, viewChild } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { GlzTabComponent } from './tab.component';

@Component({
  imports: [GlzTabComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<glz-tab label="Events"><p>Every event</p></glz-tab>`,
})
class HostComponent {
  readonly tab = viewChild.required(GlzTabComponent);
}

describe('GlzTabComponent', () => {
  it('keeps its content for the tabs to show, rendering nothing itself', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const tab = fixture.componentInstance.tab();
    expect(tab.label()).toBe('Events');
    expect(tab.content()).toBeDefined();
    expect((fixture.nativeElement as HTMLElement).textContent).not.toContain('Every event');
  });
});
