import { LiveAnnouncer } from '@angular/cdk/a11y';
import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import { expectAccessible } from '../../testing/axe';
import { GlzWorkingComponent } from './working.component';

@Component({
  imports: [GlzWorkingComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-working testId="plain" />
    <glz-working testId="marked" label="Thinking…">
      <span glzWorkingMark data-testid="mark">g</span>
    </glz-working>
  `,
})
class HostComponent {}

describe('GlzWorkingComponent', () => {
  it('says work is going on, announces it politely, and takes a mark of its own', async () => {
    const announce = vi.fn(() => Promise.resolve());
    TestBed.configureTestingModule({
      providers: [{ provide: LiveAnnouncer, useValue: { announce } }],
    });
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const plain = root.querySelector('[data-testid="plain"]');
    expect(root.querySelector('[data-testid="plain-label"]')?.textContent).toBe('Working…');
    expect(plain?.querySelector('.dot')).not.toBeNull();
    expect(plain?.querySelector('.mark')?.getAttribute('aria-hidden')).toBe('true');
    const marked = root.querySelector('[data-testid="marked"]');
    expect(marked?.querySelector('[data-testid="mark"]')).not.toBeNull();
    expect(marked?.querySelector('.dot')).toBeNull();
    expect(announce).toHaveBeenCalledWith('Thinking…', 'polite');
    expect(announce).toHaveBeenCalledWith('Working…', 'polite');
    await expectAccessible(root);
  });
});
