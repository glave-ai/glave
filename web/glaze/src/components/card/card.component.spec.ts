import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzCardComponent } from './card.component';

@Component({
  imports: [GlzCardComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-card surface="raised" data-testid="card">
      <h3 glzCardHeader>Usage</h3>
      <p>1.2M tokens</p>
      <div glzCardFooter><button type="button">Details</button></div>
    </glz-card>
  `,
})
class HostComponent {}

describe('GlzCardComponent', () => {
  it('lays out header, body and footer on its surface', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const card = (fixture.nativeElement as HTMLElement).querySelector('[data-testid="card"]');
    expect(card?.classList).toContain('glz-surface-raised');
    expect(card?.querySelector('.header h3')?.textContent).toBe('Usage');
    expect(card?.querySelector('.footer button')).not.toBeNull();
    expect(card?.querySelector('.body')?.textContent).toContain('1.2M tokens');
    await expectAccessible(fixture.nativeElement);
  });
});
